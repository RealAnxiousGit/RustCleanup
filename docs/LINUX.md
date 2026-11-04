# Keeping the Linux build in step with Windows

RustCleanup is one codebase. The UI (`src/app.rs`, `src/ui/`, `src/events.rs`,
`src/settings.rs`, `src/i18n.rs`) is shared, and only the code that talks to
the operating system is split. Read this before porting a Windows change, and
work through the checklist at the end for every new feature.

## How the split works

`src/sys/mod.rs` compiles the Windows modules in `src/sys/*.rs` only with
`#[cfg(windows)]`. On Linux it compiles `src/sys/linux/*.rs` instead and
re-exports them under the same names:

```rust
#[cfg(target_os = "linux")]
pub use linux::{chkdsk, command, disk, memory, ...};
```

The UI always writes `crate::sys::memory::...`, so each Linux file must have
**the same public functions, types and signatures** as its Windows twin. When
a Windows function changes signature, the Linux one must change with it, or
the Linux build stops compiling.

| Windows module (`src/sys/`) | Linux twin (`src/sys/linux/`) | What the Linux side uses |
|---|---|---|
| `chkdsk.rs` | `chkdsk.rs` | `fsck -n` and its per-filesystem tools, `btrfs scrub`, `xfs_scrub` |
| `command.rs` | `command.rs` | `std::process::Command`, no console window flags |
| `disk.rs` | `disk.rs` | `/proc/self/mountinfo` and `statvfs` |
| `memory.rs` | `memory.rs` | `/proc/meminfo`, `process_madvise(MADV_PAGEOUT)`, `memory.reclaim` in the user's cgroup, `drop_caches` |
| `power.rs` | `power.rs` | `powerprofilesctl`, `/sys/devices/system/cpu` (root) |
| `privilege.rs` | `privilege.rs` | `geteuid() == 0` |
| `process_actions.rs` | `process_actions.rs` | `kill`, `SIGSTOP`/`SIGCONT`, `setpriority` |
| `process_monitor.rs` | `process_monitor.rs` | `/proc/<pid>` |
| `registry.rs` | `registry.rs` | `~/.config/RustCleanup/settings.conf` |
| `shell.rs` | `shell.rs` | `xdg-open` run as the user, `app_exe()` (AppImage aware) |
| `single_instance.rs` | `single_instance.rs` | lock file plus a Unix socket (`show`, `quit`) |
| `startup.rs` | `startup.rs` | `~/.config/autostart/rustcleanup.desktop` |
| `storage_page.rs` | `storage_page.rs` | `baobab`, `filelight` or `qdirstat` |
| `tools.rs` | `tools.rs` | desktop apps found on `PATH` |
| `tray.rs` | `tray.rs` | `ksni` (StatusNotifierItem) and `notify-rust` |
| `video.rs` | `video.rs` | `ffmpeg` helpers |
| (none) | `user.rs` | runs things as the real user when RustCleanup is root (`pkexec`/`sudo`) |
| `copilot.rs`, `telemetry.rs`, `updates.rs` | (none) | Windows only, their switches are hidden on Linux |
| `com.rs`, `tasks.rs`, `*_tests.rs` | (none) | Windows only |
| `format.rs`, `updater.rs` | shared | `#[cfg]` blocks inside the file |

The updater is shared: on Linux it downloads the `RustCleanup-x86_64.AppImage`
release asset and its `.sha256` instead of the `.exe`.

## Rules for shared code

1. **Wording that names Windows.** Use `os()` from `src/i18n.rs` with both
   strings wrapped in `n_` so the i18n test sees them:

   ```rust
   tr(os(n_("Running as administrator"), n_("Running as root")))
   ```

   Both strings need translations in every `lang/*.txt` file. Linux text says
   root, `pkexec`, swap, page cache, desktop, panel and the system tray where
   Windows says administrator, UAC, page file, standby list and taskbar.
2. **Windows-only UI.** Put `#[cfg(windows)]` on the widget or the whole
   section (Copilot, Telemetry and Updates are examples). If a variable is then
   unused on Linux, add `#[cfg(not(windows))] let _ = &variable;` rather than
   changing the Windows code.
3. **Restarting or relaunching RustCleanup.** Use `crate::sys::shell::app_exe()`,
   never `std::env::current_exe()`. Inside an AppImage the current exe is a
   temporary mount, and `app_exe()` returns the `.AppImage` file instead.
4. **Opening links, folders and apps.** Go through `sys::shell`. On Linux it
   drops from root to the real user, so a browser never runs as root.
5. **Files the user owns.** When RustCleanup runs as root, files it writes in
   the user's home must be handed back with `sys::user::give_to_user`.
6. **Fonts.** `settings::FontChoice::files()` returns file names on Windows and
   fontconfig patterns on Linux. A new font choice needs both.
7. **No em dashes** in any UI text, on either system.

## Checklist for each new Windows feature

- [ ] Does the feature call a new function in `src/sys/X.rs`? Add the same
      function to `src/sys/linux/X.rs`, or `#[cfg(windows)]` the UI that calls it.
- [ ] Is it a new `sys` module? Add the Linux file, list it in
      `src/sys/linux/mod.rs` and in the `pub use linux::{...}` line, or keep the
      module and its UI behind `#[cfg(windows)]`.
- [ ] Does any new UI text mention Windows, UAC, administrator, Explorer, the
      taskbar, the registry, the page file or a Windows tool? Wrap it in `os()`
      and write the Linux wording.
- [ ] New `Job` variants that only Windows sends get
      `#[cfg_attr(not(windows), allow(dead_code))]` in `src/events.rs`.
- [ ] Translate every new string in all 11 languages, including the Linux ones.
- [ ] Windows: `cargo check --all-targets` and `cargo test`.
- [ ] Linux (WSL Ubuntu, building in `~/rc`, not `/mnt/c`):
      `cargo check --all-targets`, `cargo test --release` and
      `cargo build --release`, then open the app and look at the new feature.
- [ ] Release: build both, put the AppImage, the `.tar.gz` and their `.sha256`
      files next to the Windows `.exe` in `dist/Version X.Y.Z/`, and attach all
      of them to the GitHub release.

## Building on Linux

```bash
sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev \
  libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev \
  libdbus-1-dev fontconfig
cargo build --release
```

That build only runs on distros with the same or a newer glibc than the build
machine. **Release builds** target glibc 2.31 instead, so one binary runs on
Ubuntu 20.04+, Linux Mint 20+, Debian 11+, Fedora 32+, openSUSE Leap 15.3+
and Arch:

```bash
python3 -m venv ~/zigenv && ~/zigenv/bin/pip install ziglang
cargo install cargo-zigbuild
PATH=~/zigenv/bin:$PATH cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.31
# binary: target/x86_64-unknown-linux-gnu/release/RustCleanup
objdump -T target/x86_64-unknown-linux-gnu/release/RustCleanup | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1
```

The last line must print `GLIBC_2.31` or lower. The binary links only libc,
libm, libpthread and libdl; X11, Wayland, Vulkan, OpenGL and D-Bus are loaded
at run time, so nothing else has to match.

**AppImage:** put the binary in `usr/bin/` of an AppDir with an `AppRun` that
execs it, `rustcleanup.desktop` and `rustcleanup.png` (from `assets/icon.png`)
at the top, squash it with `mksquashfs AppDir app.squashfs -root-owned -comp zstd`,
and prepend AppImage's static runtime
(`https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64`):
`cat runtime-x86_64 app.squashfs > RustCleanup-x86_64.AppImage`. The static
runtime needs no `libfuse2`. Keep the asset name `RustCleanup-x86_64.AppImage`,
because the updater downloads it by that name. The `.tar.gz` holds the binary,
the icon, the `.desktop` file and `LICENSE`. Write a `.sha256` next to each with
`sha256sum`.

Optional at run time: `pkexec` (Restart as root), `baobab`, `filelight` or `qdirstat` (Manage Storage),
`power-profiles-daemon` (Low Power Mode), `ffmpeg` (the Changelog video),
`fonts-noto-cjk` (Japanese and Chinese).
