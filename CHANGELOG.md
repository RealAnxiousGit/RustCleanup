# Changelog

All notable changes to RustCleanup are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

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
