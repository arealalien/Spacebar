use serde::Serialize;
use std::{
    collections::HashMap,
    mem::size_of,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Devices::DeviceAndDriverInstallation::*,
    Foundation::{CloseHandle, GetLastError, ERROR_NO_MORE_ITEMS, INVALID_HANDLE_VALUE, MAX_PATH},
    Storage::FileSystem::{
        CreateFileW, GetDriveTypeW, GetLogicalDrives, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    },
    System::{
        Ioctl::{GUID_DEVINTERFACE_DISK, IOCTL_STORAGE_GET_DEVICE_NUMBER, STORAGE_DEVICE_NUMBER},
        Registry::REG_SZ,
        SystemInformation::GetWindowsDirectoryW,
        WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE},
        IO::DeviceIoControl,
    },
};

#[derive(Serialize)]
pub struct UsbDrive {
    id: String,
    label: String,
    letters: Vec<String>,
}

struct DiskInterfaces(HDEVINFO);

impl Drop for DiskInterfaces {
    fn drop(&mut self) {
        unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

fn device_id(node: u32) -> Option<String> {
    let mut value = [0u16; MAX_DEVICE_ID_LEN as usize];
    (unsafe { CM_Get_Device_IDW(node, value.as_mut_ptr(), value.len() as u32, 0) } == CR_SUCCESS)
        .then(|| String::from_utf16_lossy(value.split(|&ch| ch == 0).next().unwrap_or_default()))
}

fn device_label(node: u32) -> String {
    for property in [CM_DRP_FRIENDLYNAME, CM_DRP_DEVICEDESC] {
        let mut value = [0u16; 256];
        let mut size = size_of::<[u16; 256]>() as u32;
        let mut kind = 0;
        let status = unsafe {
            CM_Get_DevNode_Registry_PropertyW(
                node,
                property,
                &mut kind,
                value.as_mut_ptr().cast(),
                &mut size,
                0,
            )
        };
        if status == CR_SUCCESS && kind == REG_SZ {
            let text =
                String::from_utf16_lossy(value.split(|&ch| ch == 0).next().unwrap_or_default());
            if !text.trim().is_empty() {
                return text;
            }
        }
    }
    "USB drive".into()
}

fn ejectable_usb_ancestor(mut node: u32) -> Option<u32> {
    let mut removable = None;
    let required = DN_STARTED | DN_REMOVABLE;
    loop {
        let id = device_id(node)?.to_ascii_uppercase();
        let mut status = 0;
        let mut problem = 0;
        if unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, node, 0) } == CR_SUCCESS
            && status & required == required
        {
            removable = Some(node);
        }
        if id.starts_with("USB\\") {
            // Stop at the USB peripheral or its composite function, before any shared hub.
            return if id.starts_with("USB\\VID_") {
                removable
            } else {
                None
            };
        }
        let mut parent = 0;
        if unsafe { CM_Get_Parent(&mut parent, node, 0) } != CR_SUCCESS {
            return None;
        }
        node = parent;
    }
}

fn disk_number(path: *const u16) -> Option<u32> {
    unsafe {
        let handle = CreateFileW(
            path,
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut number = STORAGE_DEVICE_NUMBER::default();
        let mut bytes = 0;
        let result = DeviceIoControl(
            handle,
            IOCTL_STORAGE_GET_DEVICE_NUMBER,
            null(),
            0,
            (&mut number as *mut STORAGE_DEVICE_NUMBER).cast(),
            size_of::<STORAGE_DEVICE_NUMBER>() as u32,
            &mut bytes,
            null_mut(),
        );
        CloseHandle(handle);
        (result != 0 && bytes as usize >= size_of::<STORAGE_DEVICE_NUMBER>())
            .then_some(number.DeviceNumber)
    }
}

fn mounted_drives() -> HashMap<u32, Vec<String>> {
    let drive_mask = unsafe { GetLogicalDrives() };
    let mut drives = HashMap::<_, Vec<_>>::new();
    for (index, letter) in (b'A'..=b'Z').enumerate() {
        if drive_mask & (1 << index) == 0 {
            continue;
        }
        let root = [u16::from(letter), b':' as u16, b'\\' as u16, 0];
        if !matches!(
            unsafe { GetDriveTypeW(root.as_ptr()) },
            DRIVE_FIXED | DRIVE_REMOVABLE
        ) {
            continue;
        }
        let path: Vec<_> = format!(r"\\.\{}:", char::from(letter))
            .encode_utf16()
            .chain([0])
            .collect();
        if let Some(number) = disk_number(path.as_ptr()) {
            drives
                .entry(number)
                .or_default()
                .push(format!("{}:", char::from(letter)));
        }
    }
    drives
}

fn windows_drive() -> Option<String> {
    let mut directory = [0u16; MAX_PATH as usize];
    let length = unsafe { GetWindowsDirectoryW(directory.as_mut_ptr(), directory.len() as u32) };
    (length >= 2 && directory[1] == b':' as u16)
        .then(|| String::from_utf16_lossy(&directory[..2]).to_ascii_uppercase())
}

struct Disk {
    node: u32,
    eject_node: u32,
    number: Option<u32>,
}

fn usb_disk(set: HDEVINFO, interface: &SP_DEVICE_INTERFACE_DATA) -> Option<Disk> {
    unsafe {
        let mut bytes = 0;
        SetupDiGetDeviceInterfaceDetailW(set, interface, null_mut(), 0, &mut bytes, null_mut());
        if (bytes as usize) < size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() {
            return None;
        }
        // Allocate the variable-length interface path with alignment for the detail header.
        let mut buffer = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
        let detail = buffer
            .as_mut_ptr()
            .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
        (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
        let mut info = SP_DEVINFO_DATA {
            cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };
        if SetupDiGetDeviceInterfaceDetailW(set, interface, detail, bytes, null_mut(), &mut info)
            == 0
        {
            return None;
        }
        let eject_node = ejectable_usb_ancestor(info.DevInst)?;
        let number = disk_number(std::ptr::addr_of!((*detail).DevicePath).cast());
        Some(Disk {
            node: info.DevInst,
            eject_node,
            number,
        })
    }
}

fn removable_drives() -> Result<Vec<(u32, UsbDrive)>, String> {
    let set = unsafe {
        SetupDiGetClassDevsW(
            &GUID_DEVINTERFACE_DISK,
            null(),
            null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    };
    if set == INVALID_HANDLE_VALUE as HDEVINFO {
        return Err("Couldn’t read the connected USB drives.".into());
    }
    let set = DiskInterfaces(set);

    let mut drives = HashMap::<u32, UsbDrive>::new();
    let mut volumes = None;
    for index in 0.. {
        let mut interface = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        if unsafe {
            SetupDiEnumDeviceInterfaces(
                set.0,
                null(),
                &GUID_DEVINTERFACE_DISK,
                index,
                &mut interface,
            )
        } == 0
        {
            if unsafe { GetLastError() } != ERROR_NO_MORE_ITEMS {
                return Err("Couldn’t read the connected USB drives.".into());
            }
            break;
        }
        let Some(disk) = usb_disk(set.0, &interface) else {
            continue;
        };
        let Some(id) = device_id(disk.eject_node) else {
            continue;
        };
        let drive = drives.entry(disk.eject_node).or_insert_with(|| UsbDrive {
            id,
            label: device_label(disk.node),
            letters: Vec::new(),
        });
        if let Some(letters) = disk
            .number
            .and_then(|number| volumes.get_or_insert_with(mounted_drives).get(&number))
        {
            drive.letters.extend(letters.iter().cloned());
        }
    }

    let system = windows_drive();
    let mut drives: Vec<_> = drives
        .into_iter()
        .filter(|(_, drive)| {
            !system
                .as_ref()
                .is_some_and(|letter| drive.letters.contains(letter))
        })
        .collect();
    for (_, drive) in &mut drives {
        drive.letters.sort_unstable();
        drive.letters.dedup();
    }
    drives.sort_by(|(_, left), (_, right)| {
        (&left.label, &left.letters, &left.id).cmp(&(&right.label, &right.letters, &right.id))
    });
    Ok(drives)
}

pub fn list() -> Result<Vec<UsbDrive>, String> {
    Ok(removable_drives()?
        .into_iter()
        .map(|(_, drive)| drive)
        .collect())
}

pub fn eject(id: &str) -> Result<(), String> {
    // Only accept a currently listed USB drive, even if the caller passes another PnP ID.
    let (node, _) = removable_drives()?
        .into_iter()
        .find(|(_, drive)| drive.id == id)
        .ok_or("This USB drive is no longer available to eject.")?;
    let mut veto = PNP_VetoTypeUnknown;
    // A veto buffer keeps Windows feedback in Spacebar's menu.
    let mut veto_name = [0u16; MAX_PATH as usize];
    let result = unsafe {
        CM_Request_Device_EjectW(
            node,
            &mut veto,
            veto_name.as_mut_ptr(),
            veto_name.len() as u32,
            0,
        )
    };
    if result == CR_SUCCESS {
        return Ok(());
    }
    Err(match result {
        CR_NO_SUCH_DEVNODE | CR_DEVICE_NOT_THERE => {
            "This USB drive is no longer available to eject."
        }
        _ if veto == PNP_VetoAlreadyRemoved => "This USB drive is no longer available to eject.",
        _ if result == CR_ACCESS_DENIED || veto == PNP_VetoInsufficientRights => {
            "Windows didn’t allow Spacebar to eject this drive."
        }
        _ if [
            PNP_VetoPendingClose,
            PNP_VetoWindowsApp,
            PNP_VetoWindowsService,
            PNP_VetoOutstandingOpen,
        ]
        .contains(&veto) =>
        {
            "This drive is in use. Close any files or apps using it and try again."
        }
        _ => "Windows couldn’t safely eject this drive.",
    }
    .into())
}
