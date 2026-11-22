use crate::desktop::handle;
use serde::Deserialize;
use std::{
    cell::RefCell,
    collections::HashMap,
    mem::size_of,
    ptr::{null, null_mut},
    slice,
    time::Instant,
};
use tauri::WebviewWindow;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
    Graphics::Gdi::{
        ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
        GetMonitorInfoW, MonitorFromWindow, SelectObject, AC_SRC_ALPHA, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
        MONITORINFO, MONITOR_DEFAULTTONEAREST,
    },
    UI::{
        HiDpi::GetDpiForWindow,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const SUBCLASS: usize = 2;
const SNOW_TIMER: usize = 0x534e4f57;
const HEADROOM: f32 = 42.0;

#[derive(Clone, Copy, Deserialize, PartialEq)]
pub(crate) struct SnowAppearance {
    radius: f32,
    light: bool,
    motion: bool,
}

thread_local! {
    static ROOFS: RefCell<HashMap<usize, Roof>> = RefCell::default();
}

#[tauri::command]
pub(crate) fn set_surface_snow(
    source: WebviewWindow,
    appearance: Option<SnowAppearance>,
) -> Result<(), String> {
    if !matches!(
        source.label(),
        "settings" | "system" | "widget-popup" | "folder" | "context"
    ) {
        return Err("Snow is unavailable on this surface.".into());
    }
    let owner = handle(&source)? as usize;
    source
        .run_on_main_thread(move || {
            let owner = owner as HWND;
            if let Some(appearance) = appearance {
                if let Err(error) = attach(owner, appearance) {
                    eprintln!("Couldn't add window snow. {error}");
                }
            } else {
                detach(owner);
            }
        })
        .map_err(|error| error.to_string())
}

fn attach(owner: HWND, appearance: SnowAppearance) -> std::io::Result<()> {
    ROOFS.with_borrow_mut(|roofs| {
        let key = owner as usize;
        if let Some(roof) = roofs.get_mut(&key) {
            roof.appearance = appearance;
            roof.sync(owner, true);
            return Ok(());
        }
        let mut roof = Roof::new(owner, appearance)?;
        if unsafe { SetWindowSubclass(owner, Some(snow_proc), SUBCLASS, 0) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        roof.sync(owner, true);
        roofs.insert(key, roof);
        Ok(())
    })
}

fn detach(owner: HWND) {
    unsafe {
        KillTimer(owner, SNOW_TIMER);
        RemoveWindowSubclass(owner, Some(snow_proc), SUBCLASS);
    }
    let _ = ROOFS.try_with(|roofs| {
        if let Ok(mut roofs) = roofs.try_borrow_mut() {
            roofs.remove(&(owner as usize));
        }
    });
}

unsafe extern "system" fn snow_proc(
    owner: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        detach(owner);
        return DefSubclassProc(owner, message, wparam, lparam);
    }
    let timer = message == WM_TIMER && wparam == SNOW_TIMER;
    let result = if timer {
        0
    } else {
        DefSubclassProc(owner, message, wparam, lparam)
    };
    if timer
        || matches!(
            message,
            WM_WINDOWPOSCHANGED | WM_SHOWWINDOW | WM_SIZE | WM_DPICHANGED | WM_DISPLAYCHANGE
        )
    {
        ROOFS.with(|roofs| {
            if let Ok(mut roofs) = roofs.try_borrow_mut() {
                if let Some(roof) = roofs.get_mut(&(owner as usize)) {
                    roof.sync(owner, timer);
                }
            }
        });
    }
    result
}

#[derive(Clone, Copy, PartialEq)]
struct ArtSize {
    width: i32,
    scale: f32,
    radius: f32,
    light: bool,
}

struct Roof {
    hwnd: HWND,
    appearance: SnowAppearance,
    art: Option<Artwork>,
    position: Option<(i32, i32)>,
    visible: bool,
    animating: bool,
    started: Instant,
}

impl Roof {
    fn new(owner: HWND, appearance: SnowAppearance) -> std::io::Result<Self> {
        let style = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
        let hwnd = unsafe {
            CreateWindowExW(
                style,
                windows_sys::w!("STATIC"),
                windows_sys::w!("Spacebar Snow"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                owner,
                null_mut(),
                null_mut(),
                null(),
            )
        };
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self {
            hwnd,
            appearance,
            art: None,
            position: None,
            visible: false,
            animating: false,
            started: Instant::now(),
        })
    }

    fn sync(&mut self, owner: HWND, repaint: bool) {
        let Some((origin, width)) = visible_top(owner) else {
            self.animate(owner, false);
            if self.visible {
                unsafe { ShowWindow(self.hwnd, SW_HIDE) };
                self.visible = false;
            }
            return;
        };
        let dpi = unsafe { GetDpiForWindow(owner) };
        let scale = if dpi == 0 {
            1.0
        } else {
            (dpi as f32 / 96.0).clamp(0.5, 5.0)
        };
        let size = ArtSize {
            width,
            scale,
            radius: self.appearance.radius.clamp(0.0, 36.0),
            light: self.appearance.light,
        };
        let fresh = self.art.as_ref().is_none_or(|art| art.size != size);
        if fresh {
            self.art = Artwork::new(size);
        }
        let Some(art) = self.art.as_mut() else {
            return;
        };
        let position = POINT {
            x: origin.x,
            y: origin.y - art.headroom,
        };
        if repaint || fresh || !self.visible {
            let elapsed = self
                .appearance
                .motion
                .then(|| self.started.elapsed().as_secs_f32());
            if !art.paint(self.hwnd, position, elapsed) {
                return;
            }
        } else if self.position != Some((position.x, position.y)) {
            unsafe {
                UpdateLayeredWindow(
                    self.hwnd,
                    null_mut(),
                    &position,
                    null(),
                    null_mut(),
                    null(),
                    0,
                    null(),
                    0,
                )
            };
        }
        self.position = Some((position.x, position.y));
        if !self.visible || unsafe { IsWindowVisible(self.hwnd) } == 0 {
            self.visible = show_above(self.hwnd, owner);
        }
        self.animate(owner, self.appearance.motion);
    }

    fn animate(&mut self, owner: HWND, motion: bool) {
        if self.animating == motion {
            return;
        }
        self.animating = if motion {
            unsafe { SetTimer(owner, SNOW_TIMER, 33, None) != 0 }
        } else {
            unsafe { KillTimer(owner, SNOW_TIMER) };
            false
        };
    }
}

impl Drop for Roof {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.hwnd) };
    }
}

fn visible_top(owner: HWND) -> Option<(POINT, i32)> {
    let mut client = RECT::default();
    let mut origin = POINT::default();
    let mut monitor = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        if IsWindowVisible(owner) == 0
            || IsIconic(owner) != 0
            || GetClientRect(owner, &mut client) == 0
            || ClientToScreen(owner, &mut origin) == 0
            || GetMonitorInfoW(
                MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            ) == 0
        {
            return None;
        }
    }
    let width = client.right - client.left;
    (width > 0 && origin.y > monitor.rcMonitor.top + 1).then_some((origin, width))
}

fn show_above(hwnd: HWND, owner: HWND) -> bool {
    unsafe {
        let above = GetWindow(owner, GW_HWNDPREV);
        let topmost = |window| GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0;
        let mut flags =
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW | SWP_NOOWNERZORDER;
        let after = if above == hwnd {
            flags |= SWP_NOZORDER;
            hwnd
        } else if !above.is_null() && topmost(above) == topmost(owner) {
            above
        } else if topmost(owner) {
            HWND_TOPMOST
        } else {
            HWND_TOP
        };
        SetWindowPos(hwnd, after, 0, 0, 0, 0, flags) != 0
    }
}

struct Artwork {
    size: ArtSize,
    headroom: i32,
    bitmap: Bitmap,
    bank: Vec<u32>,
    tops: Vec<f32>,
}

impl Artwork {
    fn new(size: ArtSize) -> Option<Self> {
        let ArtSize {
            width,
            scale,
            radius,
            light,
        } = size;
        let headroom = (HEADROOM * scale).ceil() as i32;
        let height = headroom + ((radius * 0.46 + 2.0) * scale).ceil() as i32;
        let bitmap = Bitmap::new(width, height)?;
        let mut bank = vec![0; bitmap.len()];
        let mut tops = vec![headroom as f32; width as usize];
        let bottom_tint = if light {
            [169, 195, 218]
        } else {
            [189, 211, 232]
        };
        let radius = radius.min(width as f32 / (2.0 * scale));
        let inset = radius * 0.23;
        for (column, top) in tops.iter_mut().enumerate() {
            let x = (column as f32 + 0.5) / scale;
            let edge = x.min(width as f32 / scale - x);
            if edge < inset {
                continue;
            }
            let shoulder = (radius - edge).max(0.0);
            let rim = radius - (radius * radius - shoulder * shoulder).sqrt();
            let taper = (0.22 + (edge - inset) / 10.0).min(1.0);
            let drift =
                (x * 0.067).sin() * 1.8 + (x * 0.023 + 0.7).sin() * 1.4 + (x * 0.17).cos() * 0.55;
            let depth = (10.1 + drift) * taper * scale;
            let base = headroom as f32 + (rim + 0.7 + 0.33 * (x * 0.093).sin()) * scale;
            *top = base - depth;
            for row in top.floor().max(0.0) as i32..(base.ceil() as i32).min(height) {
                let row_y = row as f32;
                let coverage = ((row_y + 1.0).min(base) - row_y.max(*top)).clamp(0.0, 1.0);
                let shade = ((row_y + 0.5 - *top) / depth).clamp(0.0, 1.0).powf(1.6);
                let white = [252.0, 254.0, 255.0];
                let color = std::array::from_fn(|index| {
                    white[index] * (1.0 - shade) + bottom_tint[index] as f32 * shade
                });
                bank[row as usize * width as usize + column] = pixel(color, coverage);
            }
        }
        Some(Self {
            size,
            headroom,
            bitmap,
            bank,
            tops,
        })
    }

    fn paint(&mut self, hwnd: HWND, position: POINT, elapsed: Option<f32>) -> bool {
        let pixels = self.bitmap.pixels();
        pixels.copy_from_slice(&self.bank);
        if let Some(elapsed) = elapsed {
            draw_flakes(pixels, &self.tops, self.size, elapsed);
        }
        self.bitmap.present(hwnd, position)
    }
}

fn pixel([red, green, blue]: [f32; 3], coverage: f32) -> u32 {
    let alpha = (coverage * 255.0).round() as u32;
    let channel = |color: f32| (color * (alpha as f32 / 255.0)).round() as u32;
    alpha << 24 | channel(red) << 16 | channel(green) << 8 | channel(blue)
}

fn draw_flakes(pixels: &mut [u32], tops: &[f32], size: ArtSize, time: f32) {
    let width = size.width as usize;
    let count = (width as f32 / (55.0 * size.scale))
        .round()
        .clamp(4.0, 14.0) as usize;
    let color = if size.light {
        [190.0, 215.0, 235.0]
    } else {
        [246.0, 251.0, 255.0]
    };
    for index in 0..count {
        let i = index as f32;
        let phase = ((time + (index * 17 % 29) as f32 * 0.21)
            / (3.4 + (index * 13 % 21) as f32 * 0.18))
            .fract();
        let start = (i + 0.5 + (i * 2.4).sin() * 0.32) / count as f32 * width as f32;
        let x = start + (phase * 3.2 + i).sin() * size.scale * 5.0;
        let radius = (0.9 + (index * 11 % 13) as f32 / 15.0) * size.scale;
        let target = tops[x.clamp(0.0, (width - 1) as f32) as usize] - radius;
        let y = 1.0 + phase * target.max(1.0);
        let opacity = (phase * 4.0).min((1.0 - phase) * 3.0).min(0.85);
        for row in ((y - radius - 1.0).floor().max(0.0) as usize)
            ..((y + radius + 1.0).ceil() as usize).min(pixels.len() / width)
        {
            for column in ((x - radius - 1.0).floor().max(0.0) as usize)
                ..((x + radius + 1.0).ceil() as usize).min(width)
            {
                let distance = (column as f32 + 0.5 - x).hypot(row as f32 + 0.5 - y);
                let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0) * opacity;
                let dest = &mut pixels[row * width + column];
                if *dest == 0 && coverage > 0.0 {
                    *dest = pixel(color, coverage);
                }
            }
        }
    }
}

struct Bitmap {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u32,
    size: SIZE,
}

impl Bitmap {
    fn new(width: i32, height: i32) -> Option<Self> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            if dc.is_null() {
                return None;
            }
            let mut bits = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bitmap.is_null() {
                DeleteDC(dc);
                return None;
            }
            let previous = SelectObject(dc, bitmap);
            if previous.is_null() || bits.is_null() {
                DeleteDC(dc);
                DeleteObject(bitmap);
                return None;
            }
            Some(Self {
                dc,
                bitmap,
                previous,
                bits: bits.cast(),
                size: SIZE {
                    cx: width,
                    cy: height,
                },
            })
        }
    }

    fn len(&self) -> usize {
        self.size.cx as usize * self.size.cy as usize
    }

    fn pixels(&mut self) -> &mut [u32] {
        // The selected top-down DIB exposes contiguous 32-bit pixels for its lifetime.
        unsafe { slice::from_raw_parts_mut(self.bits, self.len()) }
    }

    fn present(&self, hwnd: HWND, position: POINT) -> bool {
        let blend = BLENDFUNCTION {
            BlendOp: 0,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            UpdateLayeredWindow(
                hwnd,
                null_mut(),
                &position,
                &self.size,
                self.dc,
                &POINT::default(),
                0,
                &blend,
                ULW_ALPHA,
            ) != 0
        }
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}
