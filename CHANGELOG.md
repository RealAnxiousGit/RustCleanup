# Changelog

All notable changes to RustCleanup are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [2.0.0] - 2026-11-01

A redesign, plus a tray icon and much lower memory use.

### Added
- **Tray icon** with the app's icon. Closing the window now hides RustCleanup to the notification area instead of quitting. Right-click the icon for **Open App**, **Clean RAM**, **Open task list**, **Credits** and **Exit**; clicking the icon opens the app. The icon comes back on its own if Explorer restarts (for example after the Copilot switch).
- **Notifications while hidden.** When RustCleanup is minimized or in the tray, finished jobs, warnings and errors from the Activity log also appear as Windows notifications.
- **Virtual Memory** section under Clean RAM: page file size, usage and peak for each page file, plus the system commit charge. If no page file is configured, it says so.
- **Credits** window (button above the Activity log), with links to the toolkits, the libraries and the GitHub repository. Links open in your browser without admin rights.
- **Theme button** next to Credits. It shows a sun in dark mode ("Light Mode") and a crescent moon in light mode ("Dark Mode"). The switch fades over about a quarter of a second, or happens instantly if Windows animation effects are off. On first launch the app follows your Windows app theme, then remembers your choice. Window title bars follow the theme too.
- `DESIGN.md`, which records the design direction and the reason for each visual choice.
- `docs/graphify/`, a code graph of the source (made with graphify) that was used to plan the new module layout.

### Changed
- **Redesigned interface**, following the anti-slop rules: Windows 11's own fonts (Segoe UI Variable, with Consolas for console output), one accent colour used only for the main action, no emoji or generic icons, and plain explanations on every switch. Both themes were checked for WCAG AA contrast.
- **Physical Memory** (renamed from the progress-bar text) now shows RAM in use, usable and installed RAM, and available and cached memory, matching Task Manager's figures.
- **Scan Disk, the task list and Credits open as real windows** that you can drag anywhere on the desktop, instead of panels drawn inside the main window.
- **New app icon**: a memory module on an oxide tile, replacing the lightning bolt.
- The source is split into one module per feature, guided by the code graph. The old `tweaks.rs`, `util.rs`, `processes.rs` and the 900-line `app.rs` were each doing several unrelated jobs.

### Fixed
- **Scan Disk window kept growing** on its own and couldn't be resized smaller. Its output area now takes exactly the space the window has.
- **High memory use.** The renderer moved from wgpu to OpenGL (glow). Measured on the same PC: about 94 MB working set at idle, versus 318 MB in 1.0.0. With the task list open it stays around 108 MB.
- **Task list performance.** Only the rows on screen are drawn, and sorting and filtering are recalculated only when the data or your filter changes, not on every frame.
- **Online scan on exFAT/FAT32 drives.** `chkdsk /scan` only works on NTFS, so the option is now disabled for other file systems, with a note explaining why.
- Copy no longer uses em dashes, and log messages are shorter and plainer.

## [1.0.0] - 2026-11-01

First release.

### Added
- **Native GUI** built with egui/eframe, dark theme, live activity log. All slow work runs on background threads so the window never freezes.
- **Administrator manifest** (`requireAdministrator`) embedded in the exe, so Windows shows the UAC prompt on launch.
- **Clean RAM**: trims every process working set (`EmptyWorkingSet`), flushes the modified page list and purges the standby list (`NtSetSystemInformation`). Reports how much memory was freed. A live memory gauge updates every second.
- **Scan Disk**: drop-down of every mounted drive (label, file system, size). Runs `chkdsk` (read-only or `/scan` online scan) and streams its output in real time into a terminal-style window, including chkdsk's in-place progress line. Scans can be cancelled.
- **Disable Copilot**: sets `TurnOffWindowsCopilot` in HKCU and HKLM, hides the taskbar button, and can restart Explorer to apply it right away.
- **Disable Telemetry**: `AllowTelemetry=0` policies, disables and stops `DiagTrack` and `dmwappushservice`, and disables the data-collection scheduled tasks through a silent PowerShell run. Tasks that don't exist on your Windows build are skipped.
- **No Forced Updates**: notify-only Windows Update, no automatic reboots while signed in, and an optional 35-day pause.
- **Optimize Task**: live list of every process (App/Background, CPU %, memory) with filter and sorting. Optimize (trim memory and set Below Normal priority), Suspend, Resume and End task for selected processes. System-critical processes are protected, End task asks for confirmation, and suspended processes are resumed automatically when RustCleanup closes.
- **Low Power Mode**: creates a "RustCleanup Low Power" scheme (CPU capped at 60% plugged in / 50% on battery, turbo boost off, passive cooling) and restores your previous plan when switched off.
- **System Tools** drop-down: Power Config, Disk Cleanup, Disk Management, Performance Monitor, Task Manager, Resource Monitor, Command Prompt and Command Prompt [Admin].
- Each toggle reads its real state from the system on startup and after every change, and switching it off fully reverts the tweak.
- Single portable `RustCleanup.exe`: the C runtime is statically linked, with nothing to install.
