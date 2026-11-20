# Recora 2

A native screen recorder for Windows by LocMasters. Rust + Tauri, Windows Graphics Capture (the API OBS and Xbox Game Bar use)
with the GPU's hardware H.264 encoder, so a 2560x1600 screen costs very little CPU.

## What is in it
- **Record** a screen, one window (follows it when it moves) or an exact region; microphone + system sound; cursor on/off;
  3 s countdown; pause/resume; markers (flag a moment, find it later in the Library).
- **A floating capsule** (timer, mute mic / system sound with live level, marker, pause, stop). It is excluded from the capture itself.
- **Global shortcuts** (change them in Settings): Ctrl+Shift+R start/stop, Ctrl+Shift+P pause, Ctrl+Shift+M marker.
- **Library**: thumbnails, search, rename, recycle bin, trim (fast or frame-exact), copies as GIF, audio (MP3), vertical 9:16, smaller, WebM.
- **Never a silently short clip**: every recording is checked against the wall clock; if the screen was still at the end the last
  picture is held, and if the file is still too short Recora tells you instead of hiding it.

## Build the installer (Windows)
Push this folder to GitHub and run the **Build Recora (Windows installer)** workflow (Actions tab). It downloads ffmpeg,
builds, and uploads `Recora_2.0.0_x64-setup.exe` as an artifact.

Local build: install Rust (stable, MSVC), Node 22, put an `ffmpeg.exe` at
`src-tauri/binaries/ffmpeg-x86_64-pc-windows-msvc.exe`, then `npm install && npx tauri build`.

## Develop / test without Windows
`RECORA_MOCK=1` replaces the screen with a generated test picture. `cargo test` (in `src-tauri`) runs the pipeline tests:
pause/resume length, audio mixing with late and short tracks (loopback goes quiet when nothing plays), held final frame.
The interface can be previewed in any browser: open `ui/index.html` (a stand-in bridge supplies sample data).

Fonts: Fraunces and DM Sans (SIL OFL), in `ui/fonts`.


## App picker (Window mode)
"Choose app" opens a popup with two tabs: **Open now** (running windows, searchable) and **Start an app** (apps from the Start menu via `Get-StartApps`, including Store apps). Starting an app launches it, waits up to 20 s for its window, then selects it. If the app is already open its window is selected instead. Recently started apps are listed first. If the recorded window is closed, the recording is saved automatically.


## Version 2.1 changes
- Interface redone in a system style (platform font, one blue accent, grouped lists).
- The recording bar is now an island at the top centre: a small dot and timer at rest, controls and labels on hover, all labels for 6 s during the first three recordings.
- Shortcuts are now F9 (start/stop), F10 (pause), F8 (marker). Old Ctrl+Shift defaults are migrated.
- Hide areas: draw a rectangle over a chat; it is blurred (unreadable text, window keeps its look) in the saved video and follows the app window if the zone was drawn on one.
- Apps that block capture (Apple TV, Netflix...) get a note instead of a block.
