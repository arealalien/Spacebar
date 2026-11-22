use crate::{
    apps,
    desktop::PopupAnchor,
    previews::{self, Previews},
    system_panel,
    system_widgets::{
        press_keys, release_windows_key_without_start, replay_windows_chord, SPACEBAR_KEY_INPUT,
        WINDOWS_MENU_MASK_KEY,
    },
};
use std::{
    cell::{Cell, RefCell},
    mem::size_of,
    path::Path,
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED},
    Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
        TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
    },
    System::{
        LibraryLoader::GetModuleHandleW,
        Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
    UI::{
        Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK},
        Input::{
            GetRawInputData,
            KeyboardAndMouse::{
                GetAsyncKeyState, VK_CONTROL, VK_ESCAPE, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN,
                VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
            },
            RegisterRawInputDevices, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT,
            RIM_TYPEKEYBOARD,
        },
        WindowsAndMessaging::{
            CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
            GetForegroundWindow, GetMessageW, GetWindowRect, GetWindowThreadProcessId,
            IsWindowVisible, RegisterClassW, SetWindowsHookExW, UnhookWindowsHookEx, CHILDID_SELF,
            EVENT_OBJECT_SHOW, EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND, HWND_MESSAGE,
            KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, OBJID_WINDOW, WH_KEYBOARD_LL,
            WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_INPUT, WM_KEYDOWN, WM_KEYUP,
            WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASSW,
        },
    },
};

const OPEN_TIMEOUT: Duration = Duration::from_millis(650);
const CLICK_DISMISS_TIME: Duration = Duration::from_millis(210);
const KEYBOARD_DISMISS_TIME: Duration = Duration::from_millis(300);
const OPEN_REQUEST: u8 = 1;
const CLOSE_REQUEST: u8 = 2;
const OPEN_AFTER_NATIVE_REQUEST: u8 = 3;
const CLOSE_AFTER_NATIVE_REQUEST: u8 = 4;
static WINDOWS_TAP: AtomicBool = AtomicBool::new(false);
static SPACEBAR_START: AtomicBool = AtomicBool::new(false);
static REPLACE_WINDOWS_START: AtomicBool = AtomicBool::new(false);
static KEYBOARD_REQUEST: AtomicU8 = AtomicU8::new(0);
static WINDOWS_CHORD: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_spacebar_start(enabled: bool, replace: bool) {
    SPACEBAR_START.store(enabled, Ordering::Release);
    REPLACE_WINDOWS_START.store(enabled && replace, Ordering::Release);
    if !enabled {
        KEYBOARD_REQUEST.store(0, Ordering::Release);
    }
}

fn replacing_windows_start() -> bool {
    REPLACE_WINDOWS_START.load(Ordering::Acquire)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShortcutAction {
    Open,
    Close,
}

impl ShortcutAction {
    fn for_window(window: HWND) -> Self {
        if window.is_null() {
            Self::Open
        } else {
            Self::Close
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WinTap {
    Native,
    Spacebar(ShortcutAction),
    Captured(ShortcutAction),
}

fn windows_tap(
    enabled: bool,
    replacing: bool,
    action: ShortcutAction,
    can_replay: impl FnOnce() -> bool,
) -> WinTap {
    if !enabled {
        WinTap::Native
    } else if (replacing && action == ShortcutAction::Close) || can_replay() {
        // Spacebar owns the close gesture even if another window briefly takes focus.
        WinTap::Captured(action)
    } else {
        // Keep physical Windows shortcuts in apps where input replay is blocked.
        WinTap::Spacebar(action)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum WinEvent {
    Pass,
    Consume,
    ReplayModifier(u16),
    Tap(WinTap),
}

#[derive(Clone, Copy, Default)]
struct WindowsKey {
    down: u8,
    tap: Option<WinTap>,
}

#[derive(Clone, Copy, Default)]
struct Modifiers(u8);

impl Modifiers {
    const CONTROLS: u8 = 0b11;

    fn observe(&mut self, key: u16, down: bool) {
        let bit = match key {
            VK_CONTROL | VK_LCONTROL => 1 << 0,
            VK_RCONTROL => 1 << 1,
            VK_SHIFT | VK_LSHIFT => 1 << 2,
            VK_RSHIFT => 1 << 3,
            VK_MENU | VK_LMENU => 1 << 4,
            VK_RMENU => 1 << 5,
            _ => return,
        };
        if down {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }

    fn only_control(self) -> bool {
        self.0 != 0 && self.0 & !Self::CONTROLS == 0
    }

    fn none(self) -> bool {
        self.0 == 0
    }
}

impl WindowsKey {
    fn observe(&mut self, key: u16, down: bool, tap: Option<WinTap>) -> WinEvent {
        let mask = match key {
            VK_LWIN => 1,
            VK_RWIN => 2,
            _ => 0,
        };
        if !down {
            if self.down & mask == 0 {
                return WinEvent::Pass;
            }
            self.down &= !mask;
            return self.tap.take().map_or(WinEvent::Pass, WinEvent::Tap);
        }

        let held = self.down;
        if held & mask == 0 {
            self.down |= mask;
            if held == 0 {
                self.tap = tap;
            } else if matches!(self.tap.take(), Some(WinTap::Captured(_))) {
                // A second key turns a pending tap into a regular Windows shortcut.
                return WinEvent::ReplayModifier(if held == 1 { VK_LWIN } else { VK_RWIN });
            }
        }
        if matches!(self.tap, Some(WinTap::Captured(_))) {
            WinEvent::Consume
        } else {
            WinEvent::Pass
        }
    }
}

thread_local! {
    static WINDOWS_KEY: Cell<WindowsKey> = const { Cell::new(WindowsKey { down: 0, tap: None }) };
    static CONTROL_ESCAPE: Cell<bool> = const { Cell::new(false) };
    static MODIFIERS: Cell<Modifiers> = const { Cell::new(Modifiers(0)) };
    static HOOK_APP: RefCell<Option<AppHandle>> = const { RefCell::new(None) };
    static INPUT_TARGET: Cell<(usize, u32, bool)> = const { Cell::new((0, 0, false)) };
    static RAW_CLOSE_TAP: Cell<Option<(usize, u16)>> = const { Cell::new(None) };
}

fn integrity_level(process: HANDLE) -> Option<u32> {
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let mut info = [0usize; 32];
        let mut returned = 0;
        let read = GetTokenInformation(
            token,
            TokenIntegrityLevel,
            info.as_mut_ptr().cast(),
            std::mem::size_of_val(&info) as u32,
            &mut returned,
        );
        CloseHandle(token);
        if read == 0 {
            return None;
        }
        let sid = (*(info.as_ptr().cast::<TOKEN_MANDATORY_LABEL>())).Label.Sid;
        let last = (*GetSidSubAuthorityCount(sid)).checked_sub(1)?;
        GetSidSubAuthority(sid, u32::from(last)).as_ref().copied()
    }
}

fn can_replay_in_foreground() -> bool {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        return true;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid == std::process::id() {
        return true;
    }
    INPUT_TARGET.with(|target| {
        let (window, process_id, allowed) = target.get();
        if window == hwnd as usize && process_id == pid {
            return allowed;
        }
        static OWN_LEVEL: OnceLock<Option<u32>> = OnceLock::new();
        let own = OWN_LEVEL.get_or_init(|| integrity_level(unsafe { GetCurrentProcess() }));
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        let allowed = if process.is_null() {
            false
        } else {
            let level = integrity_level(process);
            unsafe { CloseHandle(process) };
            matches!((level, *own), (Some(level), Some(own)) if level <= own)
        };
        target.set((hwnd as usize, pid, allowed));
        allowed
    })
}

fn wake_watcher() {
    HOOK_APP.with(|handle| {
        if let Some(app) = handle.borrow().as_ref() {
            app.state::<Previews>().wake_watcher();
        }
    });
}

fn request_spacebar_start(action: ShortcutAction, wait_for_native: bool) {
    let request = match (action, wait_for_native) {
        (ShortcutAction::Open, false) => OPEN_REQUEST,
        (ShortcutAction::Close, false) => CLOSE_REQUEST,
        (ShortcutAction::Open, true) => OPEN_AFTER_NATIVE_REQUEST,
        (ShortcutAction::Close, true) => CLOSE_AFTER_NATIVE_REQUEST,
    };
    KEYBOARD_REQUEST.store(request, Ordering::Release);
    wake_watcher();
}

fn take_keyboard_request() -> Option<(ShortcutAction, bool)> {
    match KEYBOARD_REQUEST.swap(0, Ordering::AcqRel) {
        OPEN_REQUEST => Some((ShortcutAction::Open, false)),
        CLOSE_REQUEST => Some((ShortcutAction::Close, false)),
        OPEN_AFTER_NATIVE_REQUEST => Some((ShortcutAction::Open, true)),
        CLOSE_AFTER_NATIVE_REQUEST => Some((ShortcutAction::Close, true)),
        _ => None,
    }
}

unsafe extern "system" fn receive_start_input(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_INPUT {
        let start = system_panel::start_window();
        let owns_focus = !start.is_null() && GetForegroundWindow() == start;
        RAW_CLOSE_TAP.with(|tap| {
            if !replacing_windows_start() || (!owns_focus && tap.get().is_none()) {
                tap.set(None);
                return;
            }
            let mut input = RAWINPUT::default();
            let mut size = size_of::<RAWINPUT>() as u32;
            if GetRawInputData(
                lparam as _,
                RID_INPUT,
                (&mut input as *mut RAWINPUT).cast(),
                &mut size,
                size_of::<RAWINPUTHEADER>() as u32,
            ) == u32::MAX
                || input.header.dwType != RIM_TYPEKEYBOARD
            {
                return;
            }
            let keyboard = input.data.keyboard;
            let key = keyboard.VKey;
            if key >= WINDOWS_MENU_MASK_KEY {
                return;
            }
            let down = matches!(keyboard.Message, WM_KEYDOWN | WM_SYSKEYDOWN);
            if let Some((previous_start, held)) = tap.get() {
                if key == held && !down {
                    tap.set(None);
                    if owns_focus && start as usize == previous_start {
                        request_spacebar_start(ShortcutAction::Close, false);
                    }
                } else if key != held && down {
                    tap.set(None);
                    WINDOWS_CHORD.store(true, Ordering::Release);
                    wake_watcher();
                }
                return;
            }
            if !owns_focus || !down || !matches!(key, VK_LWIN | VK_RWIN) {
                return;
            }
            let held = GetAsyncKeyState(key as i32) < 0;
            let hooked = WINDOWS_KEY.with(|state| state.get().down != 0);
            let other_keys = [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
                .into_iter()
                .any(|other| other != key && GetAsyncKeyState(other as i32) < 0);
            // A spare key cancels the native Start tap while the physical modifier is down.
            // Leave the modifier held so shortcuts such as Win+E still work normally.
            if held && !hooked && !other_keys && press_keys(&[WINDOWS_MENU_MASK_KEY]).is_ok() {
                tap.set(Some((start as usize, key)));
            }
        });
    }
    DefWindowProcW(window, message, wparam, lparam)
}

unsafe fn register_start_input(module: HINSTANCE) -> HWND {
    let class: Vec<u16> = "Spacebar.StartKeyboard".encode_utf16().chain([0]).collect();
    if RegisterClassW(&WNDCLASSW {
        lpfnWndProc: Some(receive_start_input),
        hInstance: module,
        lpszClassName: class.as_ptr(),
        ..Default::default()
    }) == 0
    {
        return null_mut();
    }
    let window = CreateWindowExW(
        0,
        class.as_ptr(),
        null(),
        0,
        0,
        0,
        0,
        0,
        HWND_MESSAGE,
        null_mut(),
        module,
        null(),
    );
    if window.is_null() {
        return window;
    }
    let keyboard = RAWINPUTDEVICE {
        usUsagePage: 1, // Generic desktop controls
        usUsage: 6,     // Keyboard
        dwFlags: 0,     // Receive input only while Spacebar is in the foreground.
        hwndTarget: window,
    };
    if RegisterRawInputDevices(&keyboard, 1, size_of::<RAWINPUTDEVICE>() as u32) == 0 {
        DestroyWindow(window);
        return null_mut();
    }
    window
}

// Some(true) requests one toggle. Some(false) consumes a repeat or matching release.
fn control_escape(
    captured: &mut bool,
    down: bool,
    enabled: bool,
    only_control: bool,
) -> Option<bool> {
    if !down {
        return std::mem::take(captured).then_some(false);
    }
    if *captured {
        return Some(false);
    }
    if enabled && only_control {
        *captured = true;
        return Some(true);
    }
    None
}

unsafe extern "system" fn observe_windows_key(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code >= 0 && data != 0 {
        let event = &*(data as *const KBDLLHOOKSTRUCT);
        if event.flags & LLKHF_INJECTED != 0 && event.dwExtraInfo == SPACEBAR_KEY_INPUT {
            return CallNextHookEx(null_mut(), code, message, data);
        }
        let key = event.vkCode as u16;
        let down = matches!(message as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        let up = matches!(message as u32, WM_KEYUP | WM_SYSKEYUP);
        if !down && !up {
            return CallNextHookEx(null_mut(), code, message, data);
        }
        let windows_key = matches!(key, VK_LWIN | VK_RWIN);
        // If Windows already received the modifier, pass its repeats and eventual release.
        if windows_key && down && GetAsyncKeyState(key as i32) < 0 {
            return CallNextHookEx(null_mut(), code, message, data);
        }
        let modifiers = MODIFIERS.with(|state| {
            let mut modifiers = state.get();
            modifiers.observe(key, down);
            state.set(modifiers);
            modifiers
        });
        let enabled = SPACEBAR_START.load(Ordering::Acquire);
        if key == VK_ESCAPE {
            let only_control = modifiers.only_control()
                && WINDOWS_KEY.with(|state| state.get().down == 0)
                && [VK_LWIN, VK_RWIN]
                    .into_iter()
                    .all(|win| GetAsyncKeyState(win as i32) >= 0);
            if let Some(toggle) = CONTROL_ESCAPE.with(|state| {
                let mut captured = state.get();
                let result = control_escape(&mut captured, down, enabled, only_control);
                state.set(captured);
                result
            }) {
                if toggle {
                    request_spacebar_start(
                        ShortcutAction::for_window(system_panel::start_window()),
                        false,
                    );
                }
                return 1;
            }
        }
        let result = WINDOWS_KEY.with(|state| {
            let mut gesture = state.get();
            if down
                && !windows_key
                && replacing_windows_start()
                && (RAW_CLOSE_TAP.with(|tap| tap.take().is_some()) || gesture.down != 0)
            {
                WINDOWS_CHORD.store(true, Ordering::Release);
                wake_watcher();
            }
            let tap = if down && windows_key && gesture.down == 0 && modifiers.none() {
                let action = ShortcutAction::for_window(system_panel::start_window());
                Some(windows_tap(
                    enabled,
                    replacing_windows_start(),
                    action,
                    can_replay_in_foreground,
                ))
            } else {
                None
            };
            let result = gesture.observe(key, down, tap);
            state.set(gesture);
            result
        });
        match result {
            WinEvent::Pass => {}
            WinEvent::Consume => return 1,
            WinEvent::ReplayModifier(win) => {
                if replay_windows_chord(win, event) {
                    return 1;
                }
            }
            WinEvent::Tap(WinTap::Captured(action)) => {
                if enabled {
                    request_spacebar_start(action, false);
                }
                return 1;
            }
            WinEvent::Tap(tap) => {
                if let WinTap::Spacebar(action) = tap {
                    if enabled {
                        let masked = release_windows_key_without_start(key);
                        request_spacebar_start(action, !masked);
                        if masked {
                            return 1;
                        }
                    }
                }
                WINDOWS_TAP.store(true, Ordering::Release);
                wake_watcher();
            }
        }
    }
    CallNextHookEx(null_mut(), code, message, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_integrity_level_used_to_check_shortcut_replay() {
        assert!(integrity_level(unsafe { GetCurrentProcess() }).is_some());
    }

    #[test]
    fn recognizes_a_single_windows_key_tap_and_leaves_shortcuts_alone() {
        let mut key = WindowsKey::default();
        let open = WinTap::Spacebar(ShortcutAction::Open);
        assert_eq!(key.observe(VK_LWIN, true, Some(open)), WinEvent::Pass);
        assert_eq!(key.observe(VK_LWIN, true, None), WinEvent::Pass);
        assert_eq!(key.observe(VK_LWIN, false, None), WinEvent::Tap(open));
        assert_eq!(key.observe(VK_RWIN, true, Some(open)), WinEvent::Pass);
        assert_eq!(key.observe(b'D' as u16, true, None), WinEvent::Pass);
        assert_eq!(key.observe(b'D' as u16, false, None), WinEvent::Pass);
        assert_eq!(key.observe(VK_RWIN, false, None), WinEvent::Pass);
        assert_eq!(key.observe(VK_LWIN, true, None), WinEvent::Pass);
        assert_eq!(key.observe(VK_LWIN, false, None), WinEvent::Pass);
        assert_eq!(key.observe(VK_LWIN, true, Some(open)), WinEvent::Pass);
        assert_eq!(key.observe(VK_RWIN, true, None), WinEvent::Pass);
        assert_eq!(key.observe(VK_LWIN, false, None), WinEvent::Pass);
        assert_eq!(key.observe(VK_RWIN, false, None), WinEvent::Pass);
    }

    #[test]
    fn opening_and_closing_start_consume_both_halves_even_after_focus_changes() {
        let mut key = WindowsKey::default();
        for win in [VK_LWIN, VK_RWIN] {
            for action in [ShortcutAction::Open, ShortcutAction::Close] {
                let tap = WinTap::Captured(action);
                assert_eq!(key.observe(win, true, Some(tap)), WinEvent::Consume);
                assert_eq!(key.observe(win, true, None), WinEvent::Consume);
                assert_eq!(key.observe(win, false, None), WinEvent::Tap(tap));
            }
        }
    }

    #[test]
    fn replacement_owns_the_close_even_when_replaying_input_is_unavailable() {
        let mut key = WindowsKey::default();
        let tap = windows_tap(true, true, ShortcutAction::Close, || false);
        assert_eq!(tap, WinTap::Captured(ShortcutAction::Close));
        assert_eq!(key.observe(VK_LWIN, true, Some(tap)), WinEvent::Consume);
        assert_eq!(key.observe(VK_LWIN, false, None), WinEvent::Tap(tap));
        assert_eq!(
            windows_tap(true, false, ShortcutAction::Close, || false),
            WinTap::Spacebar(ShortcutAction::Close)
        );
    }

    #[test]
    fn a_chord_restores_the_deferred_windows_modifier_and_cancels_the_close() {
        let mut key = WindowsKey::default();
        assert_eq!(
            key.observe(VK_RWIN, true, Some(WinTap::Captured(ShortcutAction::Close))),
            WinEvent::Consume
        );
        assert_eq!(
            key.observe(b'E' as u16, true, None),
            WinEvent::ReplayModifier(VK_RWIN)
        );
        assert_eq!(key.observe(b'E' as u16, true, None), WinEvent::Pass);
        assert_eq!(key.observe(b'E' as u16, false, None), WinEvent::Pass);
        assert_eq!(key.observe(VK_RWIN, false, None), WinEvent::Pass);
    }

    #[test]
    fn dismissing_a_native_start_leak_preserves_the_original_close() {
        let now = Instant::now();
        let mut state = Tracking {
            dismissing: Some(Dismissal {
                action: ShortcutAction::Close,
                at: now,
            }),
            shortcut: Some(RecentShortcut::new(ShortcutAction::Close, false, now)),
            ..Default::default()
        };
        assert_eq!(state.finish_dismissal(), Some(ShortcutAction::Close));
        assert!(state.shortcut.is_none());
        assert_eq!(state.finish_dismissal(), None);
    }

    #[test]
    fn native_start_activation_after_focus_loss_keeps_the_menu_closed() {
        let now = Instant::now();
        let mut state = Tracking {
            spacebar_focus_lost_at: Some(now),
            ..Default::default()
        };
        assert_eq!(state.native_action(now, false), ShortcutAction::Close);
        assert_eq!(
            state.native_action(now + OPEN_TIMEOUT, false),
            ShortcutAction::Open
        );
        state.shortcut = Some(RecentShortcut::new(ShortcutAction::Open, false, now));
        assert_eq!(state.native_action(now, false), ShortcutAction::Open);
    }

    #[test]
    fn search_host_is_only_recovered_for_a_start_handoff() {
        let now = Instant::now();
        let mut state = Tracking::default();
        assert!(!state.expects_search_host(now, true, false));
        assert!(!state.expects_search_host(now, false, true));
        assert!(state.expects_search_host(now, true, true));
        state.spacebar_focus_lost_at = Some(now);
        assert!(state.expects_search_host(now, true, false));
        state.search_shortcut_until = Some(now + OPEN_TIMEOUT);
        assert!(!state.expects_search_host(now, true, false));
        assert!(!state.expects_search_host(now, true, true));
    }

    #[test]
    fn captures_control_escape_with_spacebar_start_and_releases_it_once() {
        let mut captured = false;
        assert_eq!(control_escape(&mut captured, true, false, true), None);
        assert_eq!(control_escape(&mut captured, true, true, false), None);
        assert_eq!(control_escape(&mut captured, true, true, true), Some(true));
        assert_eq!(control_escape(&mut captured, true, true, true), Some(false));
        assert_eq!(
            control_escape(&mut captured, false, false, false),
            Some(false)
        );
        assert_eq!(control_escape(&mut captured, false, true, false), None);
    }

    #[test]
    fn modifier_events_preserve_the_other_control_key_and_reject_alt_chords() {
        let mut keys = Modifiers::default();
        keys.observe(VK_LCONTROL, true);
        keys.observe(VK_RCONTROL, true);
        keys.observe(VK_LCONTROL, false);
        assert!(keys.only_control());
        keys.observe(VK_RMENU, true);
        assert!(!keys.only_control());
        keys.observe(VK_RMENU, false);
        keys.observe(VK_RCONTROL, false);
        assert!(keys.none());
    }
}

unsafe extern "system" fn start_window_changed(
    _: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    let activates = event == EVENT_SYSTEM_FOREGROUND;
    if activates {
        RAW_CLOSE_TAP.with(|tap| {
            if let Some((start, key)) = tap.get() {
                if start != GetForegroundWindow() as usize {
                    tap.set(None);
                    if GetAsyncKeyState(key as i32) < 0 {
                        WINDOWS_CHORD.store(true, Ordering::Release);
                    }
                }
            }
        });
    }
    let reveals_foreground = object == OBJID_WINDOW
        && child == CHILDID_SELF as i32
        && !hwnd.is_null()
        && hwnd == GetForegroundWindow();
    if (activates || reveals_foreground)
        && (replacing_windows_start() || !system_panel::start_window().is_null())
    {
        wake_watcher();
    }
}

pub fn watch_keyboard(app: AppHandle) {
    thread::spawn(move || unsafe {
        HOOK_APP.with(|handle| *handle.borrow_mut() = Some(app));
        let mut modifiers = Modifiers::default();
        for key in [
            VK_LCONTROL,
            VK_RCONTROL,
            VK_LSHIFT,
            VK_RSHIFT,
            VK_LMENU,
            VK_RMENU,
        ] {
            modifiers.observe(key, GetAsyncKeyState(key as i32) < 0);
        }
        MODIFIERS.with(|state| state.set(modifiers));
        let module = GetModuleHandleW(null());
        let input = register_start_input(module);
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(observe_windows_key), module, 0);
        if hook.is_null() {
            eprintln!(
                "Couldn’t install the Start keyboard hook. {}",
                std::io::Error::last_os_error()
            );
        }
        let start_events = [
            EVENT_SYSTEM_FOREGROUND,
            EVENT_OBJECT_SHOW,
            EVENT_OBJECT_UNCLOAKED,
        ]
        .map(|event| {
            SetWinEventHook(
                event,
                event,
                null_mut(),
                Some(start_window_changed),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        });
        if hook.is_null() && input.is_null() && start_events.iter().all(|event| event.is_null()) {
            return;
        }
        let mut message = MSG::default();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            DispatchMessageW(&message);
        }
        if !hook.is_null() {
            UnhookWindowsHookEx(hook);
        }
        for event in start_events.into_iter().filter(|event| !event.is_null()) {
            UnhookWinEvent(event);
        }
        if !input.is_null() {
            DestroyWindow(input);
        }
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Host {
    Start,
    Search,
    Other,
}

#[derive(Clone, Copy)]
pub struct Panel {
    pub bounds: RECT,
}

struct RecentShortcut {
    action: ShortcutAction,
    until: Instant,
    waiting: bool,
}

impl RecentShortcut {
    fn new(action: ShortcutAction, waiting: bool, now: Instant) -> Self {
        let timeout = if waiting || action == ShortcutAction::Close {
            OPEN_TIMEOUT
        } else {
            KEYBOARD_DISMISS_TIME
        };
        Self {
            action,
            until: now + timeout,
            waiting,
        }
    }
}

struct Dismissal {
    action: ShortcutAction,
    at: Instant,
}

#[derive(Default)]
struct Tracking {
    host: Option<(usize, u32, Host)>,
    requested_at: Option<Instant>,
    seen_at: Option<Instant>,
    last_panel: Option<Panel>,
    dismissing: Option<Dismissal>,
    shortcut: Option<RecentShortcut>,
    search_shortcut_until: Option<Instant>,
    spacebar_focus_lost_at: Option<Instant>,
    cancel_on_open: bool,
}

impl Tracking {
    fn finish_dismissal(&mut self) -> Option<ShortcutAction> {
        let dismissal = self.dismissing.take()?;
        self.requested_at = None;
        self.seen_at = None;
        self.last_panel = None;
        self.cancel_on_open = false;
        self.shortcut = None;
        self.spacebar_focus_lost_at = None;
        Some(dismissal.action)
    }

    fn native_action(&self, now: Instant, spacebar_open: bool) -> ShortcutAction {
        if let Some(shortcut) = &self.shortcut {
            shortcut.action
        } else if self.closing_spacebar(now, spacebar_open) {
            ShortcutAction::Close
        } else {
            ShortcutAction::Open
        }
    }

    fn closing_spacebar(&self, now: Instant, spacebar_open: bool) -> bool {
        spacebar_open
            || self
                .spacebar_focus_lost_at
                .is_some_and(|lost| now.duration_since(lost) < OPEN_TIMEOUT)
    }

    fn expects_search_host(&self, now: Instant, replacing: bool, spacebar_open: bool) -> bool {
        if self.search_shortcut_until.is_some_and(|until| now < until) {
            return false;
        }
        // Newer Windows builds can hand Start's foreground window to SearchHost.
        self.shortcut
            .as_ref()
            .is_some_and(|shortcut| shortcut.waiting)
            || (replacing && self.closing_spacebar(now, spacebar_open))
    }

    fn panel(&mut self, now: Instant) -> Option<Panel> {
        let owned = self
            .requested_at
            .is_some_and(|at| now.duration_since(at) < OPEN_TIMEOUT)
            || self
                .seen_at
                .is_some_and(|at| now.duration_since(at) < CLICK_DISMISS_TIME);
        let hwnd = unsafe { GetForegroundWindow() };
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        let host = if let Some((_, _, host)) = self
            .host
            .filter(|&(window, process, _)| window == hwnd as usize && process == pid)
        {
            host
        } else {
            let name = apps::process_path(pid)
                .as_deref()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().to_ascii_lowercase());
            let host = match name.as_deref() {
                Some("startmenuexperiencehost.exe") => Host::Start,
                Some("searchhost.exe") => Host::Search,
                _ => Host::Other,
            };
            self.host = Some((hwnd as usize, pid, host));
            host
        };
        if host == Host::Other
            || (host == Host::Search
                && !owned
                && !self.expects_search_host(
                    now,
                    replacing_windows_start(),
                    !system_panel::start_window().is_null(),
                ))
        {
            return None;
        }

        let mut cloaked = 0u32;
        let mut bounds = RECT::default();
        let visible = unsafe {
            IsWindowVisible(hwnd) != 0
                && DwmGetWindowAttribute(
                    hwnd,
                    DWMWA_CLOAKED as u32,
                    (&mut cloaked as *mut u32).cast(),
                    4,
                ) >= 0
                && cloaked == 0
                && GetWindowRect(hwnd, &mut bounds) != 0
        };
        if !visible || bounds.right <= bounds.left || bounds.bottom <= bounds.top {
            return None;
        }
        self.requested_at = None;
        self.seen_at = Some(now);
        let panel = Panel { bounds };
        self.last_panel = Some(panel);
        Some(panel)
    }
}

#[derive(Default)]
pub struct StartMenu(Mutex<Tracking>);

impl StartMenu {
    pub(crate) fn note_spacebar_focus_loss(&self) {
        if replacing_windows_start() {
            if let Ok(mut state) = self.0.lock() {
                state.spacebar_focus_lost_at = Some(Instant::now());
            }
        }
    }

    pub fn tracking(&self) -> bool {
        WINDOWS_TAP.load(Ordering::Acquire)
            || KEYBOARD_REQUEST.load(Ordering::Acquire) != 0
            || WINDOWS_CHORD.load(Ordering::Acquire)
            || self.0.lock().is_ok_and(|state| {
                state
                    .requested_at
                    .is_some_and(|at| at.elapsed() < OPEN_TIMEOUT)
                    || state
                        .seen_at
                        .is_some_and(|at| at.elapsed() < CLICK_DISMISS_TIME)
                    || state.dismissing.is_some()
                    || state.shortcut.is_some()
            })
    }

    pub fn panel(&self) -> Option<Panel> {
        let mut state = self.0.lock().ok()?;
        let now = Instant::now();
        if WINDOWS_TAP.swap(false, Ordering::AcqRel) {
            state.requested_at = Some(now);
        }
        let panel = state.panel(now);
        let Some(panel) = panel else {
            return state
                .seen_at
                .filter(|at| at.elapsed() < CLICK_DISMISS_TIME)
                .and(state.last_panel);
        };
        if state.cancel_on_open {
            state.cancel_on_open = false;
            let _ = crate::system_widgets::press_keys(&[VK_ESCAPE]);
            return None;
        }
        Some(panel)
    }

    pub(crate) fn next_spacebar_action(&self) -> Option<ShortcutAction> {
        let mut state = self.0.lock().ok()?;
        let requested = take_keyboard_request();
        let chord = WINDOWS_CHORD.swap(false, Ordering::AcqRel);
        if !SPACEBAR_START.load(Ordering::Acquire) {
            state.shortcut = None;
            state.dismissing = None;
            state.spacebar_focus_lost_at = None;
            state.search_shortcut_until = None;
            return None;
        }
        let now = Instant::now();
        if chord {
            state.search_shortcut_until = Some(now + OPEN_TIMEOUT);
        }
        if let Some((action, waiting)) = requested {
            state.spacebar_focus_lost_at = None;
            state.search_shortcut_until = None;
            state.shortcut = Some(RecentShortcut::new(action, waiting, now));
            if let Some(dismissal) = &mut state.dismissing {
                dismissal.action = action;
            } else if !waiting {
                return Some(action);
            }
        }
        let replacing = replacing_windows_start();
        if !replacing && state.shortcut.is_none() && state.dismissing.is_none() {
            state.spacebar_focus_lost_at = None;
            state.search_shortcut_until = None;
            return None;
        }
        let visible = state.panel(now).is_some();
        let spacebar_open = !system_panel::start_window().is_null();
        let native_start = state.host.and_then(|(hwnd, _, host)| {
            (host == Host::Start
                || (host == Host::Search
                    && state.expects_search_host(now, replacing, spacebar_open)))
            .then_some(hwnd)
        });
        if let Some(dismissal) = &state.dismissing {
            if native_start.is_none() || !visible {
                return state.finish_dismissal();
            }
            if now.duration_since(dismissal.at) < OPEN_TIMEOUT {
                return None;
            }
            state.dismissing = None;
        }
        if state
            .shortcut
            .as_ref()
            .is_some_and(|shortcut| shortcut.until <= now)
        {
            let expired = state.shortcut.take()?;
            if expired.waiting && (native_start.is_none() || !visible) {
                return Some(expired.action);
            }
        }
        let hwnd = native_start?;
        if !replacing && state.shortcut.is_none() {
            return None;
        }
        let action = state.native_action(now, spacebar_open);
        if !visible {
            // The host can gain focus before Windows has displayed its window.
            state.requested_at.get_or_insert(now);
            if state.shortcut.is_none() && action == ShortcutAction::Close {
                state.shortcut = Some(RecentShortcut::new(action, false, now));
            }
            return None;
        }
        if [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
            .iter()
            .all(|&key| unsafe { GetAsyncKeyState(key as i32) >= 0 })
            && unsafe { GetForegroundWindow() } as usize == hwnd
            && press_keys(&[VK_ESCAPE]).is_ok()
        {
            state.dismissing = Some(Dismissal { action, at: now });
        }
        None
    }
}

#[tauri::command]
pub fn toggle_start_menu(
    app: AppHandle,
    source: WebviewWindow,
    start: State<'_, StartMenu>,
    anchor: Option<PopupAnchor>,
) -> Result<(), String> {
    if replacing_windows_start() {
        return match anchor {
            Some(anchor) => system_panel::open_system_panel(
                app.clone(),
                source,
                app.state(),
                system_panel::Panel::SpacebarStart,
                anchor,
            ),
            None => system_panel::toggle_start_from_keyboard(&app),
        };
    }
    previews::close_all(&app);
    system_panel::close_spacebar_start(&app);
    let now = Instant::now();
    let mut state = start.0.lock().map_err(|error| error.to_string())?;
    // An explicit Windows Start click should own the next native menu.
    take_keyboard_request();
    state.shortcut = None;
    state.dismissing = None;
    state.search_shortcut_until = None;
    state.spacebar_focus_lost_at = None;
    if state.panel(now).is_some() {
        state.cancel_on_open = false;
        crate::system_widgets::press_keys(&[VK_ESCAPE])?;
    } else if state
        .seen_at
        .take()
        .is_some_and(|at| now.duration_since(at) < CLICK_DISMISS_TIME)
    {
        // Windows can dismiss Start on mouse-down, before the widget's click fires.
        state.requested_at = None;
    } else if state
        .requested_at
        .is_some_and(|at| now.duration_since(at) < OPEN_TIMEOUT)
    {
        state.cancel_on_open = !state.cancel_on_open;
    } else {
        crate::system_widgets::press_keys(&[VK_LWIN])?;
        state.requested_at = Some(now);
        state.cancel_on_open = false;
    }
    drop(state);
    app.state::<Previews>().wake_watcher();
    Ok(())
}
