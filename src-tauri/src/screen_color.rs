use crate::{
    desktop::handle,
    system_panel::{Panel, SystemPanel},
    widgets::valid_color,
};
use std::{
    cell::Cell,
    ptr::{copy_nonoverlapping, null, null_mut},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tauri::{Manager, WebviewWindow};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    Graphics::Gdi::{GetDC, GetPixel, ReleaseDC},
    System::{
        DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
        LibraryLoader::GetModuleHandleW,
        Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
    },
    UI::{Input::KeyboardAndMouse::VK_ESCAPE, WindowsAndMessaging::*},
};

#[derive(Clone, Copy)]
enum Selection {
    Waiting,
    LeftDown,
    RightDown,
    EscapeDown,
    Picked(Option<POINT>),
}

thread_local! {
    static SELECTION: Cell<Selection> = const { Cell::new(Selection::Waiting) };
}
static PICKING: AtomicBool = AtomicBool::new(false);

struct PickerSession;
impl Drop for PickerSession {
    fn drop(&mut self) {
        PICKING.store(false, Ordering::Release);
    }
}

struct Hook(HHOOK);
impl Drop for Hook {
    fn drop(&mut self) {
        unsafe { UnhookWindowsHookEx(self.0) };
    }
}

unsafe extern "system" fn pick_mouse(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code >= 0 && data != 0 {
        let consumed = SELECTION.with(|selection| {
            let next = match (selection.get(), message as u32) {
                (Selection::Waiting | Selection::LeftDown, WM_LBUTTONDOWN) => Selection::LeftDown,
                (Selection::LeftDown, WM_LBUTTONUP) => {
                    Selection::Picked(Some((*(data as *const MSLLHOOKSTRUCT)).pt))
                }
                (Selection::Waiting | Selection::RightDown, WM_RBUTTONDOWN) => Selection::RightDown,
                (Selection::RightDown, WM_RBUTTONUP) => Selection::Picked(None),
                _ => return false,
            };
            selection.set(next);
            true
        });
        if consumed {
            return 1;
        }
    }
    CallNextHookEx(null_mut(), code, message, data)
}

unsafe extern "system" fn cancel_key(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code >= 0 && data != 0 && (*(data as *const KBDLLHOOKSTRUCT)).vkCode == u32::from(VK_ESCAPE)
    {
        let consumed = SELECTION.with(|selection| match (selection.get(), message as u32) {
            (Selection::Waiting | Selection::EscapeDown, WM_KEYDOWN | WM_SYSKEYDOWN) => {
                selection.set(Selection::EscapeDown);
                true
            }
            (Selection::EscapeDown, WM_KEYUP | WM_SYSKEYUP) => {
                selection.set(Selection::Picked(None));
                true
            }
            _ => false,
        });
        if consumed {
            return 1;
        }
    }
    CallNextHookEx(null_mut(), code, message, data)
}

fn pick(hwnd: HWND, panel: &SystemPanel) -> Result<Option<String>, String> {
    if PICKING.swap(true, Ordering::AcqRel) {
        return Err("A screen color is already being picked.".into());
    }
    let _session = PickerSession;
    let point = unsafe {
        let mut message = MSG::default();
        PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
        SELECTION.with(|selection| selection.set(Selection::Waiting));
        let module = GetModuleHandleW(null());
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(pick_mouse), module, 0);
        if mouse.is_null() {
            return Err("Windows couldn’t start the screen picker.".into());
        }
        let _mouse = Hook(mouse);
        let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(cancel_key), module, 0);
        if keyboard.is_null() {
            return Err("Windows couldn’t start the screen picker.".into());
        }
        let _keyboard = Hook(keyboard);
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
                DispatchMessageW(&message);
            }
            if let Selection::Picked(point) = SELECTION.with(Cell::get) {
                break point;
            }
            if Instant::now() >= until
                || IsWindowVisible(hwnd) == 0
                || GetForegroundWindow() != hwnd
                || panel.panel() != Some(Panel::ColorPicker)
            {
                break None;
            }
            MsgWaitForMultipleObjects(0, null(), 0, 20, QS_ALLINPUT);
        }
    };
    let Some(point) = point else { return Ok(None) };
    let color = unsafe {
        let screen = GetDC(null_mut());
        if screen.is_null() {
            return Err("Windows couldn’t read this display.".into());
        }
        let pixel = GetPixel(screen, point.x, point.y);
        ReleaseDC(null_mut(), screen);
        if pixel == u32::MAX {
            return Err("Windows couldn’t read that pixel.".into());
        }
        format!(
            "#{:02X}{:02X}{:02X}",
            pixel & 255,
            (pixel >> 8) & 255,
            (pixel >> 16) & 255
        )
    };
    Ok(Some(color))
}

fn require_picker(source: &WebviewWindow) -> Result<HWND, String> {
    if source.label() != "system"
        || source.state::<SystemPanel>().panel() != Some(Panel::ColorPicker)
    {
        return Err("Open the Color Picker widget first.".into());
    }
    handle(source)
}

#[tauri::command]
pub async fn pick_screen_color(webview: WebviewWindow) -> Result<Option<String>, String> {
    let hwnd = require_picker(&webview)? as usize;
    tauri::async_runtime::spawn_blocking(move || {
        pick(hwnd as HWND, &webview.state::<SystemPanel>())
    })
    .await
    .map_err(|error| error.to_string())?
}

struct Clipboard;
impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

#[tauri::command]
pub fn copy_screen_color(webview: WebviewWindow, color: String) -> Result<(), String> {
    let hwnd = require_picker(&webview)?;
    if !valid_color(&color) {
        return Err("Choose a valid hex color.".into());
    }
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            return Err("The clipboard is busy. Try copying again.".into());
        }
        let _clipboard = Clipboard;
        let text: Vec<u16> = color.encode_utf16().chain([0]).collect();
        let memory = GlobalAlloc(GMEM_MOVEABLE, text.len() * size_of::<u16>());
        if memory.is_null() {
            return Err("Windows couldn’t copy this color.".into());
        }
        let buffer = GlobalLock(memory).cast::<u16>();
        if buffer.is_null() {
            windows_sys::Win32::Foundation::GlobalFree(memory);
            return Err("Windows couldn’t copy this color.".into());
        }
        copy_nonoverlapping(text.as_ptr(), buffer, text.len());
        GlobalUnlock(memory);
        const UNICODE_TEXT: u32 = 13;
        if EmptyClipboard() == 0 || SetClipboardData(UNICODE_TEXT, memory).is_null() {
            windows_sys::Win32::Foundation::GlobalFree(memory);
            return Err("Windows couldn’t copy this color.".into());
        }
    }
    Ok(())
}
