use std::{env, io, os::windows::ffi::OsStrExt, path::Path, ptr::null_mut};
use tauri::WebviewWindow;
use windows::core::{w, PCWSTR};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR},
    System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
    },
};

pub(crate) const DOCK_ONLY_ARGUMENT: &str = "--dock-only";
const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_ENTRY: PCWSTR = w!("Spacebar Dock");

fn startup_command(executable: &Path) -> Result<Vec<u16>, String> {
    let suffix = format!("\" {DOCK_ONLY_ARGUMENT}");
    let mut command: Vec<_> = std::iter::once(b'"' as u16)
        .chain(executable.as_os_str().encode_wide())
        .chain(suffix.encode_utf16())
        .collect();
    if command.len() > 260 {
        return Err("Spacebar’s installation path is too long for Windows startup.".into());
    }
    command.push(0);
    Ok(command)
}

fn registration_error(status: WIN32_ERROR) -> String {
    format!(
        "Couldn’t access Windows startup settings. {}",
        io::Error::from_raw_os_error(status as i32)
    )
}

fn settings_window(source: &WebviewWindow) -> Result<(), String> {
    if source.label() != "settings" {
        return Err("Open Spacebar Settings to manage startup.".into());
    }
    Ok(())
}

#[tauri::command]
pub fn read_startup(source: WebviewWindow) -> Result<bool, String> {
    settings_window(&source)?;
    let mut size = 0;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY.0,
            RUN_ENTRY.0,
            RRF_RT_REG_SZ,
            null_mut(),
            null_mut(),
            &mut size,
        )
    };
    match status {
        ERROR_SUCCESS => Ok(true),
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(false),
        status => Err(registration_error(status)),
    }
}

#[tauri::command]
pub fn set_startup(source: WebviewWindow, enabled: bool) -> Result<bool, String> {
    settings_window(&source)?;
    let status = if enabled {
        let executable = env::current_exe().map_err(|error| error.to_string())?;
        let command = startup_command(&executable)?;
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY.0,
                RUN_ENTRY.0,
                REG_SZ,
                command.as_ptr().cast(),
                std::mem::size_of_val(command.as_slice()) as u32,
            )
        }
    } else {
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY.0, RUN_ENTRY.0) }
    };
    match status {
        ERROR_SUCCESS => Ok(enabled),
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND if !enabled => Ok(false),
        status => Err(registration_error(status)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_quotes_the_executable_and_requests_only_the_dock() {
        let executable = Path::new(r"C:\Program Files\Spacebar Æ\spacebar.exe");
        let command = startup_command(executable).unwrap();
        assert_eq!(
            String::from_utf16(&command).unwrap(),
            "\"C:\\Program Files\\Spacebar Æ\\spacebar.exe\" --dock-only\0"
        );
        assert!(startup_command(Path::new(&"A".repeat(260))).is_err());
    }
}
