# RustCleanup

A fast, native Windows 11 optimization and debloating tool written in Rust.
It ships as one portable `RustCleanup.exe` with nothing to install.

**[Download the latest release](https://github.com/RealAnxiousGit/RustCleanup/releases/latest)**

> RustCleanup changes system settings (registry policies, services, power schemes).
> Every toggle can be switched back off to restore Windows defaults, but it's still a
> good idea to create a restore point first (`Win + R` → `SystemPropertiesProtection`).

## Features

| Feature | What it does | How to undo |
|---|---|---|
| **Clean RAM** | Trims the working set of every process (`EmptyWorkingSet`), flushes the modified page list and, optionally, purges the standby list (file cache) via `NtSetSystemInformation`. Shows how much memory was freed. | Nothing to undo: Windows pages memory back in on demand. |
| **Scan Disk** | Pick any mounted drive from the drop-down and run `chkdsk`. Output streams live into a terminal window, including the progress line. Modes: **Read-only check** (`chkdsk X:`) or **Online scan** (`chkdsk X: /scan`, NTFS). | Read-only: never modifies the disk. |
| **Disable Copilot** | Sets `TurnOffWindowsCopilot = 1` under `HKCU` and `HKLM\Software\Policies\Microsoft\Windows\WindowsCopilot`, hides the taskbar button, and (optionally) restarts Explorer so it takes effect immediately. | Toggle off: the values are deleted. |
| **Disable Telemetry** | Sets `AllowTelemetry = 0` (policy and system keys), disables and stops the `DiagTrack` and `dmwappushservice` services, and disables the telemetry scheduled tasks (Compatibility Appraiser, CEIP Consolidator/UsbCeip, Autochk Proxy, DiskDiagnostic data collector, Feedback DmClient) with a silent PowerShell run. | Toggle off: policies removed, services set back to Automatic/Manual, tasks re-enabled. |
| **No Forced Updates** | Under `HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU`: `NoAutoRebootWithLoggedOnUsers = 1`, `AUOptions = 2` (notify before download and install). It can also pause updates for 35 days, the maximum Windows allows. | Toggle off: all policy and pause values removed. |
| **Optimize Task** | A live process list (App or Background, CPU %, memory) with filter and sorting. For the selected processes: **Optimize** (trim memory and set Below Normal priority), **Suspend**, **Resume**, **End task**. | Resume suspended processes; anything still suspended is resumed when RustCleanup closes. |
| **Low Power Mode** | Creates and activates a "RustCleanup Low Power" scheme based on Power Saver: max CPU 60% plugged in / 50% on battery, turbo boost off, passive cooling. | Toggle off: your previous power plan is restored and the custom scheme is deleted. |
| **System Tools** | Launchers for Power Config, Disk Cleanup, Disk Management, Performance Monitor, Task Manager, Resource Monitor, Command Prompt and Command Prompt [Admin]. | n/a |

### Safety built in
- **Protected processes.** csrss, wininit, winlogon, services, lsass, svchost, dwm, explorer, Defender and others can't be suspended or ended, and neither can RustCleanup itself.
- **End task asks first.** It confirms, and lists which programs will lose unsaved work.
- **Toggles show the real state.** Each one is read back from the system after every change, so a failed change never leaves a switch showing the wrong state.
- **Nothing is left behind.** Suspended processes are resumed and running scans are stopped when the app closes.
- **No surprise commands.** System tools are always started by their full `System32` path, never looked up on `PATH`.
- **No crash on errors.** System calls never `unwrap()`: every failure is reported in the Activity log.

## Using it

1. Download `RustCleanup.exe` from [Releases](https://github.com/RealAnxiousGit/RustCleanup/releases/latest).
2. Double-click it and choose **Yes** on the UAC prompt (Administrator rights are required).
3. Use the toggles and buttons. Everything RustCleanup does is written to the **Activity** log at the bottom. Use **Copy** to share it.

Windows SmartScreen may warn about an unsigned app: click **More info → Run anyway**.

### Notes
- Windows 11 Home and Pro treat `AllowTelemetry = 0` as "Required" (level 1), the lowest level those editions allow. Enterprise and Education honour level 0.
- Copilot policies are written to the signed-in user's `HKCU`. If you elevate with a different admin account, the HKLM policy still applies machine-wide.
- On recent Windows 11 builds Copilot is also a Store app. The policy hides and disables it; you can uninstall the app separately in Settings → Apps.
- Purging the standby list clears the file cache, so the next launch of a recently used program may be slightly slower.

## Building from source

### Prerequisites (Windows 11, x64)
1. **Rust** (stable, MSVC toolchain): install from <https://rustup.rs>, or run
   ```powershell
   winget install Rustlang.Rustup
   ```
2. **Visual Studio Build Tools** with the *Desktop development with C++* workload, which provides the MSVC linker, `rc.exe` and the Windows 10/11 SDK:
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
- `build.rs` uses `embed-resource` to compile `resources.rc`, which embeds `app.manifest` (`requireAdministrator`, PerMonitorV2 DPI awareness), the icon and version info. The build fails if the manifest can't be embedded, so you can never get an exe that silently runs without admin rights.
- `.cargo/config.toml` links the C runtime statically (`+crt-static`), so the exe doesn't need the Visual C++ Redistributable.

### Run
- Double-click `target\release\RustCleanup.exe` and accept the UAC prompt, or
- from an **Administrator** terminal: `cargo run --release`.

Running `cargo run` from a normal terminal fails with *"The requested operation requires elevation (os error 740)"*. This is expected and shows the manifest is working.

### Tests
The unit tests are compiled into an exe that carries the same manifest. To run them without elevating, tell Windows to ignore the manifest for this shell:
```powershell
$env:__COMPAT_LAYER = "RunAsInvoker"; cargo test
```

## Project layout
```
src/
├── main.rs          window setup (eframe/egui)
├── app.rs           UI layout, state, background job dispatch
├── events.rs        worker → UI messages, Logger, panic-safe job runner
├── widgets.rs       toggle switch, cards, theme colours
└── sys/             everything that talks to Windows (no UI code)
    ├── ram.rs        EmptyWorkingSet, standby list purge, memory status
    ├── disk.rs       drive enumeration, streaming chkdsk runner
    ├── tweaks.rs     Copilot, telemetry, Windows Update policies
    ├── power.rs      Low Power Mode scheme via powercfg
    ├── processes.rs  process monitor (sysinfo), optimize/suspend/resume/end
    ├── tools.rs      System Tools launchers
    ├── registry.rs   winreg helpers
    └── util.rs       hidden commands, console decoding, privileges
app.manifest         UAC requireAdministrator manifest
resources.rc         embeds manifest, icon and version info
build.rs             compiles resources.rc into the exe
```

## How it stays responsive
The UI thread only draws and reads channels. Each operation runs on its own worker thread through `Logger::spawn_job`, which reports progress as `AppEvent`s and wakes the UI with `request_repaint`. chkdsk output is read from its pipes by dedicated reader threads. The process list is sampled by a background `ProcessMonitor` thread only while the Optimize Task window is open.

## License
[MIT](LICENSE)
