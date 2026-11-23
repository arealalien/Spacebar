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

After an in-app update, Settings shows a versioned **What’s new** tab until you dismiss it. The Windows release workflow bundles the notes from the matching GitHub draft. When creating a draft, it uses a bullet list in `release-notes/vX.Y.Z.md` if available, or GitHub’s generated notes.
