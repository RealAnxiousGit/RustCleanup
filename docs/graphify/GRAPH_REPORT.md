# Graph Report - src  (2026-11-01)

## Corpus Check
- 29 files · ~16,774 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 562 nodes · 1066 edges · 23 communities (19 shown, 4 thin omitted)
- Extraction: 97% EXTRACTED · 3% INFERRED · 0% AMBIGUOUS · INFERRED: 36 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `cf829bcc`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- tray.rs
- App
- Logger
- tasks.rs
- process_monitor.rs
- chkdsk.rs
- Palette
- power.rs
- DiskPanel
- process_actions.rs
- sys/memory.rs
- Hive
- updates.rs
- Tray
- RustCleanupApp
- TrayCommand
- OwnedHandle
- MemorySnapshot
- Virtual
- .start
- NotifyKind

## God Nodes (most connected - your core abstractions)
1. `Logger` - 33 edges
2. `App` - 32 edges
3. `Palette` - 31 edges
4. `Jobs` - 27 edges
5. `RustCleanupApp` - 24 edges
6. `semibold()` - 23 edges
7. `Job` - 18 edges
8. `Hive` - 17 edges
9. `AppEvent` - 15 edges
10. `TrayCommand` - 15 edges

## Surprising Connections (you probably didn't know these)
- `open_unelevated()` --calls--> `windows_dir()`  [INFERRED]
  src/sys/shell.rs → src/sys/command.rs
- `set_copilot_disabled()` --references--> `Logger`  [EXTRACTED]
  src/sys/copilot.rs → src/events.rs
- `restart_explorer()` --calls--> `windows_dir()`  [INFERRED]
  src/sys/shell.rs → src/sys/command.rs
- `heading()` --calls--> `semibold()`  [INFERRED]
  src/ui/widgets.rs → src/ui/theme.rs
- `primary_button()` --calls--> `semibold()`  [INFERRED]
  src/ui/widgets.rs → src/ui/theme.rs

## Import Cycles
- 1-file cycle: `src/sys/power.rs -> src/sys/power.rs`

## Communities (23 total, 4 thin omitted)

### Community 0 - "tray.rs"
Cohesion: 0.03
Nodes (5): CALLBACK_MSG, ICON_ID, NIN_BALLOONUSERCLICK, NIN_KEYSELECT, NIN_SELECT

### Community 1 - "App"
Cohesion: 0.06
Nodes (17): MAIN_TITLE, Level, Error, Info, Success, Warn, LogEntry, App (+9 more)

### Community 2 - "Logger"
Cohesion: 0.08
Nodes (19): AppEvent, JobFinished, Log, State, Tray, Job, CleanRam, Copilot (+11 more)

### Community 3 - "tasks.rs"
Cohesion: 0.08
Nodes (27): fmt_bytes(), ProcInfo, ProcKind, Background, MemoryPanel, Action, End, Optimize (+19 more)

### Community 4 - "process_monitor.rs"
Cohesion: 0.06
Nodes (10): CRITICAL, monitor_lists_processes(), ProcessMonitor, ProcSnapshot, windowed_pids(), find_own_window(), open_unelevated(), show_window() (+2 more)

### Community 5 - "chkdsk.rs"
Cohesion: 0.09
Nodes (20): collect(), ScanEvent, Finished, Line, Progress, ScanHandle, spawn_reader(), splits_lines_and_progress() (+12 more)

### Community 6 - "Palette"
Cohesion: 0.10
Nodes (16): DARK, install_fonts(), lerp_color(), LIGHT, Palette, Theme, THEME_KEY, button() (+8 more)

### Community 7 - "power.rs"
Cohesion: 0.09
Nodes (26): CmdOutput, run_hidden(), run_powershell(), COPILOT_POLICY, EXPLORER_ADVANCED, set_copilot_disabled(), active_scheme(), BALANCED_GUID (+18 more)

### Community 8 - "DiskPanel"
Cohesion: 0.09
Nodes (9): ScanMode, .ALL, OnlineScan, ReadOnly, Drive, finds_system_drive(), from_wide(), list_drives() (+1 more)

### Community 9 - "process_actions.rs"
Cohesion: 0.12
Nodes (6): nt_result(), NtResumeProcess(), NtSuspendProcess(), open(), resume(), suspend()

### Community 10 - "sys/memory.rs"
Cohesion: 0.12
Nodes (9): MEMORY_FLUSH_MODIFIED_LIST, memory_list_command(), MEMORY_PURGE_STANDBY_LIST, NtQuerySystemInformation(), NtSetSystemInformation(), PageFileInfo, STATUS_INFO_LENGTH_MISMATCH, SYSTEM_MEMORY_LIST_INFORMATION (+1 more)

### Community 11 - "Hive"
Cohesion: 0.19
Nodes (9): delete_value(), describe(), get_dword(), get_string(), Hive, CurrentUser, LocalMachine, set_dword() (+1 more)

### Community 13 - "updates.rs"
Cohesion: 0.13
Nodes (9): civil_from_days(), friendly_date(), utc_iso_in_days(), PAUSE_DAYS, PAUSE_VALUES, set_forced_updates_blocked(), WU_AU_POLICY, WU_POLICY (+1 more)

### Community 14 - "Tray"
Cohesion: 0.23
Nodes (6): add_icon(), base_data(), copy_wide(), show_menu(), Tray, window_proc()

### Community 16 - "TrayCommand"
Cohesion: 0.17
Nodes (10): MENU, send(), Shared, SHARED, TrayCommand, CleanRam, Credits, Exit (+2 more)

### Community 17 - "OwnedHandle"
Cohesion: 0.22
Nodes (6): all_pids(), clean_ram(), trim_process(), enable_privilege(), is_elevated(), OwnedHandle

### Community 18 - "MemorySnapshot"
Cohesion: 0.29
Nodes (7): MemorySnapshot, performance_info(), Physical, physical(), reads_memory_figures(), snapshot(), virtual_mem()

### Community 20 - ".start"
Cohesion: 0.50
Nodes (3): create_window(), load_icon(), message_loop()

### Community 21 - "NotifyKind"
Cohesion: 0.50
Nodes (4): NotifyKind, Error, Info, Warning

## Knowledge Gaps
- **33 isolated node(s):** `Info`, `Success`, `Warn`, `Error`, `CleanRam` (+28 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 254 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **4 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Logger` connect `Logger` to `App`, `process_monitor.rs`, `Palette`, `power.rs`, `sys/memory.rs`, `updates.rs`, `OwnedHandle`?**
  _High betweenness centrality (0.219) - this node is a cross-community bridge._
- **What connects `Info`, `Success`, `Warn` to the rest of the system?**
  _33 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `tray.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.03125 - nodes in this community are weakly interconnected._
- **Why does `App` connect `App` to `Logger`, `tasks.rs`, `process_monitor.rs`, `chkdsk.rs`, `Palette`, `DiskPanel`, `Tray`, `RustCleanupApp`, `TrayCommand`, `OwnedHandle`, `NotifyKind`?**
  _High betweenness centrality (0.202) - this node is a cross-community bridge._
- **Should `App` be split into smaller, more focused modules?**
  _Cohesion score 0.06352941176470588 - nodes in this community are weakly interconnected._
- **Why does `Jobs` connect `Logger` to `DiskPanel`, `App`, `tasks.rs`, `RustCleanupApp`?**
  _High betweenness centrality (0.076) - this node is a cross-community bridge._
- **Should `Logger` be split into smaller, more focused modules?**
  _Cohesion score 0.08115942028985507 - nodes in this community are weakly interconnected._