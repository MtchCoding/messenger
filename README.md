# Lite Messenger

## Easiest way: let GitHub build it (no tools needed)
1. Make a free account at github.com and create a **New repository** (private is fine).
2. Click **uploading an existing file**, drag in everything from this folder
   (including the hidden `.github` folder), and commit.
3. Open the **Actions** tab -> wait ~10 minutes for "Build installer" to turn green.
4. Click the finished run -> download **Lite-Messenger-Installer** at the bottom -> unzip -> run the .exe.

## Build yourself instead
Install Rust + C++ Build Tools, then:  cargo install tauri-cli --version "^2"  and  cargo tauri build

## If the build fails
Most likely the webview2-com / windows versions in src-tauri/Cargo.toml don't match Tauri's.
Open the failed run, copy the error, and send it to Claude.
