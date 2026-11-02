# Changelog

All notable changes to RustCleanup are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [3.0.0] - 2026-11-02

A rebuilt task list with Restart, a Changelog window, a font picker and a smoother, roomier interface.

### Added
- **Restart** in the task list closes a program and opens it again with the same command line and folder. A program made of many processes (like a browser) is restarted once, from its top process. Programs that ran as administrator start again as administrator; the rest start with normal rights.
- **Changelog** button next to Credits. It shows this version's release notes from GitHub. After an update, it opens by itself on the first start of the new version (and only then).
- **Font** setting, under Theme: System (Segoe UI), Arial, Times New Roman, Light UI (Segoe UI Light), Comic Sans MS and more.
- **System tools**: Windows Security, Device Manager and Settings, before Command Prompt.
- **Credits**: a Promotional trailer section. Animation by Claude Opus 5.5, music "Promo" by BombinSound on Pixabay.
- More tooltips: on the main buttons, every task list action and column, the System tools Open button, the Activity log buttons and several checkboxes.

### Changed
- **Task list sorting works on every column.** Click Name, PID, Type, CPU, Memory or Status to sort by it (Type puts apps first, Status puts suspended and protected first). Click it again to reverse the order. The sorted column is highlighted and shows an arrow, and every header lights up under the cursor.
- **Task list selection works like File Explorer.** Click a task to select it, Ctrl+click to add or remove one, Shift+click to select everything in between, Ctrl+A to select all and Esc to clear. Right-click a task for its actions, and Delete ends the selection (after asking).
- **Memory turns yellow, then red** in the task list when a process uses 4%, then 10%, of your RAM. Hover the value to see its share.
- **Suspended tasks are detected from Windows itself**, so a task shows as Suspended even if another program paused it. Suspend skips tasks that already are, and Resume undoes every suspend, so a task is never left frozen. The list refreshes right after an action.
- **Clean RAM bars turn yellow above 75% and red above 90%**, Physical Memory and Virtual Memory each by their own use.
- Buttons, task rows and column headers fade in and out on hover instead of switching at once.
- More space between controls and around dividers, so sections no longer feel cramped.
- Scrolling glides briefly instead of jumping, and the task list draws its rows directly, which keeps long lists smooth.

### Fixed
- **Updating closed RustCleanup instead of restarting it.** The new version could not take over from the old one and gave up. It now starts as soon as the old one has closed. The same fix applies to the restart after changing the hardware-accelerated UI setting.
- Task list column headers did not respond to clicks.
- Disabled buttons, like Copy and Clear on an empty Activity log, kept bright text and looked clickable.

## [2.0.3] - 2026-11-02

A hotfix for crashes when closing windows.

### Fixed
- **Closing a window crashed RustCleanup.** Closing the Task list, Settings, Credits, Scan Disk or Update window could close the whole program, on PCs with Intel graphics. The window's drawing surface was thrown away while the graphics driver was still using it. RustCleanup now lets go of it first. The fix is in a patched copy of eframe, the window library, kept in `patches/eframe`.
- **An update left the old version behind** as `RustCleanup.exe.old` next to the exe. The new version now waits for the old one to finish closing and then deletes it.

## [2.0.2] - 2026-11-02

Fixes for an antivirus false positive, Command Prompt and Manage Storage.

### Fixed
- **Microsoft Defender flagged 2.0.1 as `Trojan:Win32/Bearfoos.A!ml`** and removed it. This was a machine-learning guess, set off by RustCleanup running hidden PowerShell to create its startup task, which is also what a lot of malware does. RustCleanup no longer runs PowerShell at all: Open on startup and the telemetry scheduled tasks now use the Task Scheduler API directly, and Command Prompt [Admin] starts `cmd.exe` itself.
- **Command Prompt opened the Documents folder** in File Explorer instead of a prompt. It now opens a real Command Prompt with normal (non-admin) rights, started by the desktop's Explorer.
- **Manage Storage on a non-system drive opened Disks & volumes.** It now opens that drive's own page under System, Storage, Storage used on other drives. The system drive still opens Storage. If Settings changes and the drive can't be picked, Settings stays at Storage and the Activity log says where to go.

### Changed
- An existing Open on startup task from 2.0.1 keeps working and is updated in place.
- New hand-run tests (`cargo test desktop_ -- --ignored`) open every System Tool and each drive's Settings page.

## [2.0.1] - 2026-11-02

A Settings window, 12 languages, automatic updates and a storage overview.

### Added
- **Settings window**, from the new **Settings** button between Credits and the theme button, or from **Settings** in the tray menu. It has:
  - **Language**, with a flag and the language's own name for each choice. All text is translated into English, Español, Português, Français, Deutsch, Italiano, Nederlands, Polski, Türkçe, Русский, 日本語 and 中文. On first launch RustCleanup follows your Windows display language.
  - **Start minimized**: start in the notification area instead of opening the window.
  - **Open on startup**: a Task Scheduler logon task that starts RustCleanup with admin rights, so there is no UAC prompt at sign-in. It follows the exe if you move it or it is updated.
  - **Close to the notification area**, **Theme** and **Interface size** (90% to 175%).
  - **Notifications**, to turn off the Windows notifications shown while RustCleanup is hidden.
  - **Automatically check for updates**. On: a new release is downloaded, checked against its published SHA-256 and installed by itself, then RustCleanup restarts. Off: a window shows the new version's changelog with **Update Now** and **Never Update**, and it can be closed to decide later. **Check for updates now** checks on demand.
  - **Auto-Clean RAM**, which cleans RAM by itself when memory use goes above an amount you set with a slider (50% to 95%), at most once every 2 minutes.
  - **Hardware-accelerated UI**, which switches the interface to Direct3D 12. It is smoother on some GPUs but uses about 3 times the memory (about 335 MB versus 100 MB), so it is off by default.
  - **Reset all settings**. Settings are kept in `HKCU\Software\RustCleanup`.
- **Storage overview under Scan Disk**: how much of the selected drive is used, how much is available and its total size, with a meter that turns amber above 90%.
- **Manage Storage** button next to **Run check**. It opens Windows Settings at Storage for the system drive, or Disks & volumes for other drives.
- **Single instance.** Opening RustCleanup while it is already running brings up the copy that is running instead of starting a second one.

### Changed
- **New app icon**: a white outline of a small brush on pure black.
- **Task list, Settings and Credits opened from the tray** now open on their own, without bringing up the main window. Closing them leaves RustCleanup running in the tray.
- **The tray icon stays** in the notification area the whole time RustCleanup is running, not only while it is hidden.
- The tray menu has a **Settings** item above **Credits**.

### Fixed
- **Starting minimized showed a black, empty window** for a moment. The window now stays hidden until you open it from the tray.
- **Japanese and Chinese text sat too high** next to Latin text. The fallback fonts are now lined up with Segoe UI.

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
