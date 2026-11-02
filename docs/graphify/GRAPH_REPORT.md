# Graph Report - src  (2026-11-02)

## Corpus Check
- 39 files · ~26,270 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 817 nodes · 1767 edges · 28 communities (26 shown, 2 thin omitted)
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 65 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `cc875ad2`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- tr
- tray.rs
- Logger
- trf
- updater.rs
- Palette
- process_monitor.rs
- tasks.rs
- Hive
- chkdsk.rs
- power.rs
- RustCleanupApp
- single_instance.rs
- shell.rs
- Lang
- Tray
- sys/memory.rs
- src/settings.rs
- i18n.rs
- flags.rs
- ui/mod.rs
- TrayCommand
- clean_ram
- MemorySnapshot
- ThemeMode
- snapshot

## God Nodes (most connected - your core abstractions)
1. `tr()` - 68 edges
2. `trf()` - 58 edges
3. `App` - 48 edges
4. `Logger` - 45 edges
5. `Palette` - 41 edges
6. `Jobs` - 37 edges
7. `Lang` - 34 edges
8. `RustCleanupApp` - 32 edges
9. `semibold()` - 29 edges
10. `AppEvent` - 25 edges

## Surprising Connections (you probably didn't know these)
- `describe_exit()` --calls--> `tr()`  [INFERRED]
  src/sys/chkdsk.rs → src/i18n.rs
- `set_copilot_disabled()` --calls--> `tr()`  [INFERRED]
  src/sys/copilot.rs → src/i18n.rs
- `theme_button()` --calls--> `tr()`  [INFERRED]
  src/ui/widgets.rs → src/i18n.rs
- `open_unelevated()` --calls--> `windows_dir()`  [INFERRED]
  src/sys/shell.rs → src/sys/command.rs
- `restart_explorer()` --calls--> `run_hidden()`  [INFERRED]
  src/sys/shell.rs → src/sys/command.rs

## Import Cycles
- 1-file cycle: `src/sys/power.rs -> src/sys/power.rs`
- 2-file cycle: `src/events.rs -> src/sys/updater.rs -> src/events.rs`

## Communities (28 total, 2 thin omitted)

### Community 0 - "tr"
Cohesion: 0.05
Nodes (33): MAIN_TITLE, NOTIFY_MAX_CHARS, NOTIFY_MAX_WAIT, NOTIFY_QUIET, AUTO_CLEAN_COOLDOWN, AUTO_CLEAN_POLL, check_updates(), on_launch() (+25 more)

### Community 1 - "tray.rs"
Cohesion: 0.03
Nodes (7): CALLBACK_MSG, ICON_ADDED, ICON_ID, NIN_BALLOONUSERCLICK, NIN_KEYSELECT, NIN_SELECT, RETRY_TIMER

### Community 2 - "Logger"
Cohesion: 0.06
Nodes (35): AppEvent, JobFinished, Log, Restart, State, Tray, UpdateAvailable, UpToDate (+27 more)

### Community 3 - "trf"
Cohesion: 0.07
Nodes (23): n_(), trf(), ScanMode, .ALL, OnlineScan, ReadOnly, Drive, finds_system_drive() (+15 more)

### Community 4 - "updater.rs"
Cohesion: 0.06
Nodes (19): ASSET, check(), current_version(), Handle, http_get(), install(), is_newer(), LATEST_RELEASE (+11 more)

### Community 5 - "Palette"
Cohesion: 0.09
Nodes (22): cjk_fonts(), CJK_Y_OFFSET, CURRENT, DARK, install_fonts(), lerp_color(), LIGHT, map_font() (+14 more)

### Community 6 - "process_monitor.rs"
Cohesion: 0.07
Nodes (11): nt_result(), NtResumeProcess(), NtSuspendProcess(), open(), resume(), suspend(), CRITICAL, monitor_lists_processes() (+3 more)

### Community 7 - "tasks.rs"
Cohesion: 0.09
Nodes (30): ProcInfo, ProcKind, Background, Action, End, Optimize, Resume, Suspend (+22 more)

### Community 8 - "Hive"
Cohesion: 0.07
Nodes (21): COPILOT_POLICY, EXPLORER_ADVANCED, set_copilot_disabled(), civil_from_days(), local_time_hms(), utc_iso_in_days(), delete_value(), describe() (+13 more)

### Community 9 - "chkdsk.rs"
Cohesion: 0.09
Nodes (20): collect(), describe_exit(), ScanEvent, Finished, Line, Progress, ScanHandle, spawn_reader() (+12 more)

### Community 10 - "power.rs"
Cohesion: 0.09
Nodes (28): CmdOutput, run_hidden(), run_powershell(), active_scheme(), BALANCED_GUID, disable_low_power(), enable_low_power(), low_power_active() (+20 more)

### Community 11 - "RustCleanupApp"
Cohesion: 0.12
Nodes (10): PendingNotification, RustCleanupApp, GLOW_FALLBACK_ARG, HIDDEN_ARG, main(), RESTARTED_ARG, NotifyKind, Error (+2 more)

### Community 12 - "single_instance.rs"
Cohesion: 0.13
Nodes (6): acquire(), InstanceLock, show_running_copy(), try_lock(), SHOW_MESSAGE, WINDOW_CLASS

### Community 13 - "shell.rs"
Cohesion: 0.13
Nodes (6): windows_dir(), find_own_window(), open_unelevated(), restart_explorer(), show_window(), WindowId

### Community 14 - "Lang"
Cohesion: 0.11
Nodes (15): Lang, .ALL, De, En, Es, Fr, It, Ja (+7 more)

### Community 15 - "Tray"
Cohesion: 0.18
Nodes (9): add_icon(), base_data(), copy_wide(), create_window(), load_icon(), message_loop(), show_menu(), Tray (+1 more)

### Community 16 - "sys/memory.rs"
Cohesion: 0.12
Nodes (9): MEMORY_FLUSH_MODIFIED_LIST, memory_list_command(), MEMORY_PURGE_STANDBY_LIST, NtQuerySystemInformation(), NtSetSystemInformation(), PageFileInfo, STATUS_INFO_LENGTH_MISMATCH, SYSTEM_MEMORY_LIST_INFORMATION (+1 more)

### Community 18 - "src/settings.rs"
Cohesion: 0.22
Nodes (9): AUTO_CLEAN_RANGE, get(), KEY, reset(), Settings, SETTINGS, UI_SCALES, update() (+1 more)

### Community 19 - "i18n.rs"
Cohesion: 0.20
Nodes (8): CURRENT, every_string_is_translated(), language(), marked_strings(), parse(), placeholders(), Table, table()

### Community 20 - "flags.rs"
Cohesion: 0.34
Nodes (8): horizontal(), paint(), rgb(), SIZE, star(), union_jack(), vertical(), WHITE

### Community 21 - "ui/mod.rs"
Cohesion: 0.24
Nodes (8): app_icon(), close_pressed(), closed(), focus_window(), repaint_windows(), window_builder(), window_id(), WINDOWS

### Community 22 - "TrayCommand"
Cohesion: 0.17
Nodes (11): MENU, send(), Shared, SHARED, TrayCommand, CleanRam, Credits, Exit (+3 more)

### Community 23 - "clean_ram"
Cohesion: 0.22
Nodes (6): all_pids(), clean_ram(), trim_process(), enable_privilege(), is_elevated(), OwnedHandle

### Community 24 - "MemorySnapshot"
Cohesion: 0.29
Nodes (4): MemorySnapshot, PageFile, Physical, Virtual

### Community 25 - "ThemeMode"
Cohesion: 0.29
Nodes (5): ThemeMode, .ALL, Dark, Light, System

### Community 26 - "snapshot"
Cohesion: 0.47
Nodes (5): performance_info(), physical(), reads_memory_figures(), snapshot(), virtual_mem()

## Knowledge Gaps
- **55 isolated node(s):** `Info`, `Success`, `Warn`, `Error`, `CleanRam` (+50 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 327 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `tr()` connect `tr` to `tray.rs`, `Logger`, `trf`, `updater.rs`, `Palette`, `tasks.rs`, `Hive`, `chkdsk.rs`, `power.rs`, `RustCleanupApp`, `shell.rs`, `Tray`, `sys/memory.rs`, `i18n.rs`, `clean_ram`?**
  _High betweenness centrality (0.232) - this node is a cross-community bridge._
- **Are the 11 inferred relationships involving `tr()` (e.g. with `.footer()` and `.header()`) actually correct?**
  _`tr()` has 11 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Info`, `Success`, `Warn` to the rest of the system?**
  _55 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `tr` be split into smaller, more focused modules?**
  _Cohesion score 0.05094905094905095 - nodes in this community are weakly interconnected._
- **Why does `trf()` connect `trf` to `tr`, `tray.rs`, `Logger`, `updater.rs`, `process_monitor.rs`, `tasks.rs`, `Hive`, `power.rs`, `RustCleanupApp`, `shell.rs`, `Tray`, `sys/memory.rs`, `i18n.rs`, `clean_ram`?**
  _High betweenness centrality (0.170) - this node is a cross-community bridge._
- **Are the 4 inferred relationships involving `trf()` (e.g. with `.new()` and `.pump_events()`) actually correct?**
  _`trf()` has 4 INFERRED edges - model-reasoned connections that need verification._
- **Should `tray.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.02857142857142857 - nodes in this community are weakly interconnected._