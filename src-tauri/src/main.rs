#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod apps;
#[cfg(windows)]
mod connectivity;
#[cfg(windows)]
mod desktop;
#[cfg(windows)]
mod dock_config;
#[cfg(windows)]
mod media;
#[cfg(windows)]
mod notifications;
#[cfg(windows)]
mod popups;
#[cfg(windows)]
mod previews;
#[cfg(windows)]
mod replacement;
#[cfg(windows)]
mod screen_color;
mod settings;
#[cfg(windows)]
mod widget_popup;
#[cfg(windows)]
mod start_backdrop;
#[cfg(windows)]
mod start_launcher;
#[cfg(windows)]
mod start_menu;
#[cfg(windows)]
mod startup;
#[cfg(windows)]
mod system_panel;
#[cfg(windows)]
mod system_widgets;
#[cfg(windows)]
mod taskbar;
#[cfg(windows)]
mod telemetry;
#[cfg(windows)]
mod tray_icons;
#[cfg(windows)]
mod updates;
#[cfg(windows)]
mod usb_drives;
#[cfg(windows)]
mod widget_content;
#[cfg(windows)]
mod widgets;

fn main() {
    #[cfg(windows)]
    {
        let argument = std::env::args_os().nth(1);
        if argument
            .as_deref()
            .is_some_and(|arg| arg == replacement::GUARD_ARGUMENT)
        {
            replacement::guard();
        } else {
            desktop::run(
                argument
                    .as_deref()
                    .is_some_and(|arg| arg == startup::DOCK_ONLY_ARGUMENT),
            );
        }
    }
    #[cfg(not(windows))]
    eprintln!("Spacebar's desktop dock requires Windows.");
}
