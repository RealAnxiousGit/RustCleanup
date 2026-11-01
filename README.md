# RustCleanup

A native Windows 11 tool for freeing memory, checking disks and turning off the parts of Windows you don't want.
It ships as one portable `RustCleanup.exe` with nothing to install.

**[Download the latest release](https://github.com/RealAnxiousGit/RustCleanup/releases/latest)**

![RustCleanup 2.0 main window in dark mode, showing Physical and Virtual Memory, Scan Disk, the Windows settings switches and System tools](docs/screenshot.png)

> RustCleanup changes system settings (registry policies, services, power plans).
> Every switch can be turned off again to restore the Windows default, but it's still a
> good idea to create a restore point first (`Win + R`, then `SystemPropertiesProtection`).

## Features

| Feature | What it does | How to undo |
|---|---|---|
| **Clean RAM** | Trims the working set of every process (`EmptyWorkingSet`), flushes the modified page list and, optionally, purges the standby list (file cache) via `NtSetSystemInformation`. Reports how much memory was freed. | Nothing to undo: Windows pages memory back in on demand. |
| **Physical Memory** | Live RAM in use, out of usable and installed RAM, plus available and cached memory, refreshed every second. | n/a |
| **Virtual Memory** | Page file size, usage and peak for each page file, plus the commit charge (RAM plus page file). | n/a |
| **Scan Disk** | Pick any mounted drive and run `chkdsk`. Output streams live into its own window. Modes: **Read-only check** (`chkdsk X:`) on any drive, or **Online scan** (`chkdsk X: /scan`) on NTFS drives. | Read-only: never modifies the disk. |
| **Disable Copilot** | Sets `TurnOffWindowsCopilot = 1` under `HKCU` and `HKLM\Software\Policies\Microsoft\Windows\WindowsCopilot`, hides the taskbar button, and can restart Explorer so it applies right away. | Switch off: the values are deleted. |
| **Disable Telemetry** | Sets `AllowTelemetry = 0` (policy and system keys), disables and stops `DiagTrack` and `dmwappushservice`, and disables the telemetry scheduled tasks with a silent PowerShell run. Tasks missing on your Windows build are skipped. | Switch off: policies removed, services set back to Automatic/Manual, tasks re-enabled. |
| **No Forced Updates** | Under `HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU`: `NoAutoRebootWithLoggedOnUsers = 1`, `AUOptions = 2` (ask before download and install). Can also pause updates for 35 days, the most Windows allows. | Switch off: all policy and pause values removed. |
| **Optimize Task** | A live task list (App or Background, CPU %, memory) with filter and sorting, in its own window. For the selected processes: **Optimize** (trim memory and set Below Normal priority), **Suspend**, **Resume**, **End task**. | Resume suspended processes. Anything still suspended is resumed when RustCleanup exits. |
| **Low Power Mode** | Creates and activates a "RustCleanup Low Power" plan: CPU capped at 60% plugged in and 50% on battery, turbo boost off, passive cooling. | Switch off: your previous power plan is restored and the custom plan deleted. |
| **System Tools** | Opens Power Config, Disk Cleanup, Disk Management, Performance Monitor, Task Manager, Resource Monitor, Command Prompt or Command Prompt [Admin]. | n/a |

### Tray icon and notifications
- **Closing the window hides RustCleanup** to the notification area, so it keeps running. Click the tray icon to open it again.
- **Right-click the tray icon** for **Open App**, **Clean RAM**, **Open task list**, **Credits** and **Exit**. Only **Exit** quits.
- While the window is minimized or in the tray, finished jobs, warnings and errors from the Activity log also appear as Windows notifications.

### Light and dark mode
The button next to **Credits** switches themes: a sun means "switch to Light Mode", a crescent moon means "switch to Dark Mode". The change fades over about a quarter of a second, or happens instantly if Windows animation effects are turned off. On first launch RustCleanup follows your Windows app theme, then remembers your choice.

### Safety built in
- **Protected processes.** csrss, wininit, winlogon, services, lsass, svchost, dwm, explorer, Defender and others can't be suspended or ended, and neither can RustCleanup itself. They can only be trimmed.
- **End task asks first.** It lists which programs will lose unsaved work.
- **Switches show the real state.** They are read back from Windows on launch and after every change, so a failed change never leaves a switch showing the wrong state.
- **Nothing is left behind.** Suspended processes are resumed and running scans are stopped when RustCleanup exits.
- **No surprise programs.** System tools are started by their full `System32` path, never looked up on `PATH`. Links and the normal Command Prompt open through Explorer, so they don't inherit admin rights.
- **No crash on errors.** System calls never `unwrap()`. Every failure is written to the Activity log.

## Using it

1. Download `RustCleanup.exe` from [Releases](https://github.com/RealAnxiousGit/RustCleanup/releases/latest).
2. Double-click it and choose **Yes** on the UAC prompt (Administrator rights are required).
3. Use the buttons and switches. Everything RustCleanup does is written to the **Activity** log. **Copy** puts the log on the clipboard.

Windows SmartScreen may warn about an unsigned app: click **More info**, then **Run anyway**.

### Notes
- Windows 11 Home and Pro treat `AllowTelemetry = 0` as "Required" (level 1), the lowest level those editions allow. Enterprise and Education honour level 0.
- Copilot policies are written to the signed-in user's `HKCU`. If you elevate with a different admin account, the HKLM policy still applies machine-wide.
- On recent Windows 11 builds Copilot is also a Store app. The policy hides and disables it; you can uninstall the app in Settings, Apps.
- Purging the standby list clears the file cache, so the next launch of a recently used program may be slightly slower.
- A read-only `chkdsk` of a large exFAT drive can take many minutes. The progress line in the Scan Disk window keeps updating while it runs.

## Building from source

### Prerequisites (Windows 11, x64)
1. **Rust** (stable, MSVC toolchain): install from <https://rustup.rs>, or run
   ```powershell
   winget install Rustlang.Rustup
   ```
2. **Visual Studio Build Tools** with the *Desktop development with C++* workload, which provides the MSVC linker, `rc.exe` and the Windows SDK:
   ```powershell
   winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
   ```

### Build
```powershell
git clone https://github.com/RealAnxiousGit/RustCleanup
cd RustCleanup
cargo build --release
```
The output is `target\release\RustCleanup.exe`, a single self-contained file:
- `build.rs` uses `embed-resource` to compile `resources.rc`, which embeds `app.manifest` (`requireAdministrator`, PerMonitorV2 DPI awareness), the icon and the version info. The build fails if the manifest can't be embedded, so you can never get an exe that silently runs without admin rights.
- `.cargo/config.toml` links the C runtime statically (`+crt-static`), so the exe doesn't need the Visual C++ Redistributable.
- eframe is built with the OpenGL (`glow`) renderer instead of its default wgpu renderer, which keeps memory use low.

### Run
- Double-click `target\release\RustCleanup.exe` and accept the UAC prompt, or
- from an **Administrator** terminal: `cargo run --release`.

Running `cargo run` from a normal terminal fails with *"The requested operation requires elevation (os error 740)"*. This is expected and shows the manifest is working.

### Tests
The test binary carries the same manifest. To run the tests without elevating, tell Windows to ignore the manifest for this shell:
```powershell
$env:__COMPAT_LAYER = "RunAsInvoker"; cargo test
```

## Project layout
The layout follows the code graph in [`docs/graphify/GRAPH_REPORT.md`](docs/graphify/GRAPH_REPORT.md): one module per feature, with shared plumbing kept separate. Open [`docs/graphify/graph.html`](docs/graphify/graph.html) in a browser to explore the graph. After changing code, regenerate it with `graphify update src`.

```
src/
├── main.rs             window setup (eframe/egui)
├── app.rs              wires sections, windows, tray and theme together
├── events.rs           worker-to-UI messages, Logger, Jobs (panic-safe job runner)
├── ui/                 the interface, one file per section or window
│   ├── memory.rs       Clean RAM: Physical and Virtual Memory
│   ├── disk.rs         Scan Disk and the chkdsk output window
│   ├── tasks.rs        Optimize Task and the task list window
│   ├── tweaks.rs       Windows settings switches and System tools
│   ├── credits.rs      Credits window
│   ├── activity.rs     Activity log
│   ├── theme.rs        palettes, fonts, theme fade
│   └── widgets.rs      switch, meter, buttons, theme button, links
└── sys/                everything that talks to Windows (no UI code)
    ├── memory.rs       memory figures, EmptyWorkingSet, standby purge
    ├── disk.rs         drive enumeration
    ├── chkdsk.rs       streaming chkdsk runner
    ├── copilot.rs      Copilot policy
    ├── telemetry.rs    telemetry policies, services, tasks
    ├── updates.rs      Windows Update policies and pause
    ├── power.rs        Low Power Mode plan via powercfg
    ├── process_monitor.rs  task list sampling (sysinfo)
    ├── process_actions.rs  optimize, suspend, resume, end
    ├── tray.rs         tray icon, menu, notifications (Shell_NotifyIconW)
    ├── tools.rs        System Tools launchers
    ├── shell.rs        Explorer restart, unelevated links, window helpers
    ├── command.rs      hidden console tools and output decoding
    ├── privilege.rs    elevation check, token privileges
    ├── registry.rs     winreg helpers
    └── format.rs       sizes and dates
DESIGN.md               design direction and the reason for each choice
app.manifest            UAC requireAdministrator manifest
resources.rc            embeds manifest, icon and version info
build.rs                compiles resources.rc into the exe
```

## How it stays responsive
The UI thread only draws and reads channels. Each operation runs on its own worker thread through `Jobs::start`, which reports progress as `AppEvent`s and wakes the UI with `request_repaint`. The window repaints once a second for the live memory figures and otherwise only when something changes. chkdsk output is read by dedicated reader threads, and the task list is sampled every 2 seconds by a background thread only while its window is open. The tray icon has its own thread and message loop, so its menu works while the main window is hidden.

## Credits
- Programming: Claude Opus 5.5
- Prompt Engineer: Nicholas Slaughter
- Toolkits: [graphify](https://github.com/Graphify-Labs/graphify) (code graph used to plan the module layout) and [anti-slop](https://github.com/miqdadbadjuber/anti-slop) (design rules applied to the interface)
- Built with [egui/eframe](https://github.com/emilk/egui), [sysinfo](https://github.com/GuillaumeGomez/sysinfo), [windows-rs](https://github.com/microsoft/windows-rs), [winreg](https://github.com/gentoo90/winreg-rs) and [embed-resource](https://github.com/nabijaczleweli/rust-embed-resource)

## License
[MIT](LICENSE)
