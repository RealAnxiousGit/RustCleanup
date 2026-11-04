//! System Tools dropdown: quick launchers for the desktop's own utilities.
//!
//! Linux desktops ship different apps for the same job, so each tool lists
//! the GNOME, KDE, Xfce and other versions and starts the first one that is
//! installed. Everything but the root terminal runs as the desktop user.

use std::process::{Command, Stdio};

use super::command::which;
use super::privilege::is_elevated;
use super::user::{self, user_command};
use crate::i18n::n_;

/// One way to start a tool: a program and its arguments. A program starting
/// with `@` is a console program, run inside a terminal.
type Candidate = (&'static str, &'static [&'static str]);

/// A launchable tool.
pub struct Tool {
    pub label: &'static str,
    pub description: &'static str,
    candidates: &'static [Candidate],
    /// Run with RustCleanup's root rights (only the root terminal).
    root: bool,
}

/// Terminals, with the flag that makes them run a command.
const TERMINALS: [(&str, &str); 10] = [
    ("x-terminal-emulator", "-e"),
    ("gnome-terminal", "--"),
    ("ptyxis", "--"),
    ("kgx", "--"),
    ("konsole", "-e"),
    ("xfce4-terminal", "-x"),
    ("mate-terminal", "-x"),
    ("lxterminal", "-e"),
    ("alacritty", "-e"),
    ("xterm", "-e"),
];

pub const TOOLS: [Tool; 10] = [
    Tool {
        label: n_("System Monitor"),
        description: n_("Processes, performance and resources"),
        candidates: &[
            ("gnome-system-monitor", &[]),
            ("plasma-systemmonitor", &[]),
            ("ksysguard", &[]),
            ("mate-system-monitor", &[]),
            ("xfce4-taskmanager", &[]),
            ("lxtask", &[]),
            ("@htop", &[]),
            ("@top", &[]),
        ],
        root: false,
    },
    Tool {
        label: n_("Disk Usage Analyzer"),
        description: n_("See what fills each drive"),
        candidates: &[("baobab", &[]), ("filelight", &[]), ("qdirstat", &[]), ("@ncdu", &["/"])],
        root: false,
    },
    Tool {
        label: n_("Disks"),
        description: n_("Drives, partitions and SMART health"),
        candidates: &[("gnome-disks", &[]), ("partitionmanager", &[]), ("@lsblk", &["-f"])],
        root: false,
    },
    Tool {
        label: n_("Power Settings"),
        description: n_("Power profiles, battery and sleep"),
        candidates: &[
            ("gnome-control-center", &["power"]),
            ("systemsettings", &["kcm_powerdevilprofilesconfig"]),
            ("xfce4-power-manager-settings", &[]),
            ("mate-power-preferences", &[]),
            ("cinnamon-settings", &["power"]),
        ],
        root: false,
    },
    Tool {
        label: n_("System Logs"),
        description: n_("The system journal"),
        candidates: &[("gnome-logs", &[]), ("ksystemlog", &[]), ("@journalctl", &["-e"])],
        root: false,
    },
    Tool {
        label: n_("Software Updates"),
        description: n_("Install updates and apps"),
        candidates: &[
            ("gnome-software", &["--mode", "updates"]),
            ("plasma-discover", &["--mode", "update"]),
            ("update-manager", &[]),
            ("mintupdate", &[]),
            ("pamac-manager", &["--updates"]),
        ],
        root: false,
    },
    Tool {
        label: n_("Hardware Information"),
        description: n_("Hardware and drivers"),
        candidates: &[
            ("hardinfo2", &[]),
            ("hardinfo", &[]),
            ("kinfocenter", &[]),
            ("gnome-control-center", &["system"]),
            ("@inxi", &["-F"]),
            ("@lshw", &["-short"]),
        ],
        root: false,
    },
    Tool {
        label: n_("Settings"),
        description: n_("Desktop settings"),
        candidates: &[
            ("gnome-control-center", &[]),
            ("systemsettings", &[]),
            ("xfce4-settings-manager", &[]),
            ("cinnamon-settings", &[]),
            ("mate-control-center", &[]),
            ("lxqt-config", &[]),
        ],
        root: false,
    },
    Tool {
        label: n_("Terminal"),
        description: n_("Standard (non-root) terminal"),
        candidates: &[("@", &[])],
        root: false,
    },
    Tool {
        label: n_("Terminal [Root]"),
        description: n_("Root terminal"),
        candidates: &[("@", &[])],
        root: true,
    },
];

fn terminal() -> Option<(&'static str, &'static str)> {
    TERMINALS.into_iter().find(|(t, _)| which(t).is_some())
}

impl Tool {
    /// Launches the first installed version of the tool without waiting.
    pub fn launch(&self) -> Result<(), String> {
        let home = user::desktop_user().home.clone();
        let (program, args) = self
            .candidates
            .iter()
            .find(|(p, _)| match p.strip_prefix('@') {
                Some("") => terminal().is_some(),
                Some(console) => which(console).is_some() && terminal().is_some(),
                None => which(p).is_some(),
            })
            .ok_or_else(|| format!("no app for {} is installed", self.label))?;

        let mut cmd = match program.strip_prefix('@') {
            Some(console) => {
                let (term, exec) = terminal().ok_or("no terminal is installed")?;
                let mut cmd = self.command(term);
                if self.root && !is_elevated() {
                    cmd.args([exec, "sudo", "-i"]);
                } else if !console.is_empty() {
                    cmd.arg(exec).arg(console).args(*args);
                }
                cmd
            }
            None => {
                let mut cmd = self.command(program);
                cmd.args(*args);
                cmd
            }
        };
        cmd.current_dir(if self.root && is_elevated() { "/root".into() } else { home })
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        super::shell::spawn_detached(&mut cmd).map_err(|e| format!("could not start {}: {e}", self.label))
    }

    fn command(&self, program: &str) -> Command {
        let path = which(program).unwrap_or_else(|| program.into());
        if self.root { Command::new(path) } else { user_command(path) }
    }
}
