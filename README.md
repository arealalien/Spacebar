# Spacebar

Spacebar is a customizable Windows 11 dock built with React, TypeScript, Tauri v2, and Rust. It runs on the primary monitor and hides the Windows taskbar while it is open by default.

## Run on Windows

The project needs Node.js, the Rust MSVC toolchain, Microsoft C++ Build Tools with **Desktop development with C++**, and Microsoft Edge WebView2. See the [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows).

From this directory in PowerShell:

```powershell
npm.cmd ci
npm.cmd run tauri dev
```

The launcher detects the local Rust toolchain in `.toolchain` if present. Otherwise it uses Cargo from your `PATH`.

To try the settings and a sample dock in the browser, run `npm.cmd run dev` and open `http://127.0.0.1:1420`. To create a Windows installer, run `npm.cmd run tauri build`. Tauri writes the NSIS installer under `src-tauri/target/release/bundle/nsis/`.

## Install and update Spacebar

Download the Windows x64 setup `.exe` from the [latest Spacebar release](https://github.com/arealalien/Spacebar/releases/latest) and run it. Spacebar 0.6.8 and later check for updates without a GitHub account or token. If you use an earlier version, run the latest installer once to switch to the public update feed.

Spacebar checks the latest published release when it starts, even when it starts into the dock at sign-in. When a newer version is available, a notice with a **Download** button appears above the version label in Settings. Once the download has passed signature verification, **Install update** starts the installer and restarts Spacebar.

## Publish a release

Set the Actions secret `TAURI_SIGNING_PRIVATE_KEY` to the private key that matches the public key in `src-tauri/tauri.conf.json`. Set `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if the key has a password. Keep the signing key backed up outside the repository. Installed copies use the matching public key to verify updates.

For each release, bump the version in `package.json`, `package-lock.json`, `src-tauri/tauri.conf.json`, and the `spacebar` entries in `src-tauri/Cargo.toml` and `src-tauri/Cargo.lock`. Commit the version changes and push a matching tag such as `v0.6.9`. The Windows release workflow builds and signs the installer, uploads it with its update manifest, then publishes the release.

## What the dock supports

- In **Dock layout → Placement**, choose the bottom, top, left, or right edge. **Full width** spans a horizontal edge, and **Full height** spans a side edge. Alignment positions items along the selected edge in either mode. Previews, folders, and auto-hide follow the selected edge.

- Open a Windows app to add it to the dock. Windows for pinned apps appear on the existing icon. Click an app icon to open, focus, or restore it. Click its icon again while it is active to minimize that window. Spacebar Settings appears in the App Icons row while its window is open.
- Spacebar follows the live Explorer taskbar order for apps it can match. If Explorer withholds the live list, Spacebar uses saved Windows pin order and keeps running apps in launch order. Custom pins and some Windows pins may still differ.
- Drag an app icon within its row to change the order. Spacebar saves that order and restores an app’s position when it reopens. When an icon has keyboard focus, use **Alt+Left/Right** or **Alt+Up/Down** on a side dock.
- Hover an open app to see its windows in an animated thumbnail panel. Hover a thumbnail to see the full-size preview at the window’s location. Under **Dock layout → App feedback**, switch **Full-size preview location** to **Dock’s monitor** to see it on the dock’s display instead. Click a thumbnail to switch to that window, or middle-click it to close the window. Closed pins show their app name.
- Under **Dock layout → App feedback**, set **Notification badges** to **Dot**, **Nothing**, or **Dot with number**. Spacebar uses the active badge information an app provides to Windows. Apps that expose unread activity without a count show a dot.
- Appearance includes a setting to hide the settings icon and an option to hide the whole dock until the pointer reaches the bottom screen edge. The reveal can use easing or an adjustable bounce. The dock updates as settings are saved and respects the system reduced-motion preference.
- Right-click any app icon or widget to open Settings or Widgets. Middle-click an app icon or choose its name from the menu to launch it or open a new window if the app supports one. For running apps, **Close app** asks each window to close and **End task** force-closes the processes that own those windows. End task is unavailable for protected or shared processes and Spacebar Settings. Unpin a pinned app from its menu. A running app stays visible until its last window closes. Spacebar saves its pins separately from Windows.
- Closing Settings leaves the dock running. Choose **Quit Spacebar** from the dock menu to restore the Windows taskbar, or turn off **Replace Windows taskbar** under **Appearance** to show both at once. A separate process restores the Windows taskbar if the dock exits unexpectedly.
- Open **Saved docks** to name and save your current settings, widgets, and app order. Choose **Use dock** to load a saved configuration. After editing it, choose **Save changes** to update the saved copy.

## Widgets

Under **Appearance**, **Tinted** is the default dock material. Dock and widget backgrounds in Tinted mode follow the tint color. The tint also colors app previews and running indicators. The Settings accent changes the Settings interface and its preview wallpaper.

Under **App icons**, choose **Original**, **Rounded corners**, or **Circular**. Keep the default transparent background or add a tinted, black, white, or custom-colored tile behind the icons. Click effects play when you click an app to open, restore, or minimize it. Set **Click speed** from 50% to 200% and try it in the dock preview.

Right-click a widget to choose **Small**, **Normal**, or **Large**. Use the color pickers or enter hex colors in the same menu. Widgets such as System Load and Network have a separate picker for each accent. Choose **Default** beside a color to reset it. **Icon size** under **Dock layout** sets the base scale, and **Clear** in Appearance removes the dock background.

The **Digital clock** combines the Plain and Outline clocks. Turn on **Show frame** for an outline and **Show background** for a filled face.

In **Widgets → System**, **Windows Start** opens the native Start menu. **Spacebar Start** opens a launcher centered along the dock with app search, Calendar, Spotify, and system shortcuts. Click its widget, tap the Windows key, or press Ctrl+Esc to toggle it. Enable **Replace Windows Start menu** to also route Windows Start buttons to Spacebar Start. Shut down and Restart ask for confirmation.

Open **Widgets** in Settings or from the dock menu. Drag a Clock, Spotify, Start menu, Folder, or App Icons tile from the library onto the dock. A preview follows your cursor. Drag existing widgets to rearrange them. Drag any app icon while the library is open to move the whole App Icons row, or hold **Shift** while dragging outside the library.

Drag an individual app onto a folder with the library closed, or drop a widget onto the folder at any time. Open the folder to rename it, add apps, and move items back out. Click the same Folder, System Tray, Quick Settings, or Notifications widget again to close its panel. Right-click a widget with a surface to toggle **Show background** and **Show frame** independently. Frameless widgets use a 3 px inset on docks with a background. Remove a widget with its red minus button while the library is open, or choose **Remove widget** from its context menu.

From the **Layout** section of the widget library, add a **Spacer** for an empty gap or a **Divider** for a line between sections. Right-click a Spacer and set **Spacer size** in pixels. Dividers run across the dock on any screen edge.

Spotify Artwork, Track, Mini, and Text share the Spotify desktop app’s playback session. Hover any of them for album art, track and artist, and playback controls. Spotify Mini shows its own play and pause button on hover or keyboard focus. The widget library also has Flip and Twilight clocks, a Day Progress widget, paired CPU and RAM rings, live system readings, audio output and microphone controls, and Windows shortcuts. Network usage counts data transferred since monitoring began, across active physical adapters. GPU readings depend on the Windows driver exposing performance counters.

System Tray shows registered notification-area app icons in a Spacebar panel. Apps can expose different tray actions, and some icons may only support opening an existing window. Quick Settings includes live speaker and mic controls and links to Windows settings. System panels use the dock tint in Tinted mode. The Notifications panel includes a calendar and a shortcut to the Windows notification history. The Screenshots widget uses Windows’ region snip and full-screen save shortcuts and can open Snipping Tool.
