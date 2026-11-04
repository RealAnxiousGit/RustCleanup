//! Tray icon (StatusNotifierItem, through ksni) with the same right-click
//! menu as on Windows, and desktop notifications (through notify-rust).
//!
//! The icon needs a StatusNotifierItem host: KDE, Xfce, Cinnamon, MATE and
//! most panels have one; GNOME needs the AppIndicator extension. Without a
//! host `Tray::start` fails, and the app then quits on close as on Windows.
//!
//! The "show yourself" socket from `single_instance` is served here too, so
//! a second launch brings this copy's window back.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use ksni::blocking::TrayMethods;

use super::single_instance;
use super::user::{self, user_command};
use crate::i18n::{n_, tr};

/// What the user picked in the tray menu (or by clicking the icon).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    OpenApp,
    CleanRam,
    OpenTaskList,
    Settings,
    Credits,
    Exit,
}

/// Severity of a notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NotifyKind {
    Info,
    Warning,
    Error,
}

const MENU: [(&str, TrayCommand); 6] = [
    (n_("Open App"), TrayCommand::OpenApp),
    (n_("Clean RAM"), TrayCommand::CleanRam),
    (n_("Open task list"), TrayCommand::OpenTaskList),
    (n_("Settings"), TrayCommand::Settings),
    (n_("Credits"), TrayCommand::Credits),
    (n_("Exit"), TrayCommand::Exit),
];

const ICON_PNG: &[u8] = include_bytes!("../../../assets/icon.png");

type OnCommand = Arc<dyn Fn(TrayCommand) + Send + Sync>;

struct Item {
    on_command: OnCommand,
    icon: Vec<ksni::Icon>,
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        "rustcleanup".into()
    }

    fn title(&self) -> String {
        "RustCleanup".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "RustCleanup".into(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        (self.on_command)(TrayCommand::OpenApp);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        let mut items = Vec::new();
        for (i, (label, cmd)) in MENU.into_iter().enumerate() {
            // A line between the actions and Exit, as on Windows.
            if i == MENU.len() - 1 {
                items.push(ksni::MenuItem::Separator);
            }
            items.push(
                StandardItem {
                    label: tr(label).to_string(),
                    activate: Box::new(move |this: &mut Self| (this.on_command)(cmd)),
                    ..Default::default()
                }
                .into(),
            );
        }
        items
    }
}

/// The app icon as ARGB32 (network byte order), as StatusNotifierItem wants.
fn icon_pixmap() -> Vec<ksni::Icon> {
    match eframe::icon_data::from_png_bytes(ICON_PNG) {
        Ok(icon) => {
            let mut data = icon.rgba;
            for px in data.chunks_exact_mut(4) {
                px.rotate_right(1);
            }
            vec![ksni::Icon {
                width: icon.width as i32,
                height: icon.height as i32,
                data,
            }]
        }
        Err(_) => Vec::new(),
    }
}

/// Serves "show" requests from later launches, for the life of the process.
fn serve_show_requests(on_command: OnCommand) {
    let Some(listener) = single_instance::take_listener() else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("single-instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut line = String::new();
                if BufReader::new(stream).read_line(&mut line).is_err() {
                    continue;
                }
                match line.trim() {
                    "show" => on_command(TrayCommand::OpenApp),
                    // The root copy from "Restart as root" is taking over.
                    "quit" => on_command(TrayCommand::Exit),
                    _ => {}
                }
            }
        });
}

/// The app icon as a file, for notifications (they take a path or a theme
/// icon name, and an AppImage installs no theme icon).
fn icon_file() -> Option<&'static PathBuf> {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        let dir = user::desktop_user().runtime_dir();
        let dir = if dir.is_dir() { dir } else { std::env::temp_dir() };
        let path = dir.join("rustcleanup-icon.png");
        std::fs::write(&path, ICON_PNG).ok()?;
        user::give_to_user(&path);
        Some(path)
    })
    .as_ref()
}

/// A live tray icon. Dropping it removes the icon.
pub struct Tray {
    handle: Option<ksni::blocking::Handle<Item>>,
}

impl Tray {
    /// Creates the tray icon. Fails when the desktop has no tray host.
    pub fn start(on_command: impl Fn(TrayCommand) + Send + Sync + 'static) -> Result<Self, String> {
        let on_command: OnCommand = Arc::new(on_command);
        serve_show_requests(on_command.clone());
        let item = Item {
            on_command,
            icon: icon_pixmap(),
        };
        // The session bus belongs to the desktop user, so connect as them.
        let handle = user::with_user_identity(move || {
            item.disable_dbus_name(false)
                .spawn()
                .map_err(|e| e.to_string())
        })?;
        Ok(Self {
            handle: Some(handle),
        })
    }

    /// Shows a desktop notification.
    pub fn notify(&self, kind: NotifyKind, title: &str, body: &str) {
        let (title, body) = (title.to_string(), body.to_string());
        // Sending can wait on D-Bus, so it never holds up the window.
        std::thread::spawn(move || notify(kind, &title, &body));
    }

    /// Removes the icon now. Used before a hard exit, where Drop won't run.
    pub fn remove(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.shutdown().wait();
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
    }
}

fn notify(kind: NotifyKind, title: &str, body: &str) {
    let urgency = match kind {
        NotifyKind::Info => notify_rust::Urgency::Low,
        NotifyKind::Warning => notify_rust::Urgency::Normal,
        NotifyKind::Error => notify_rust::Urgency::Critical,
    };
    let icon = icon_file().map(|p| p.to_string_lossy().into_owned());
    let mut n = notify_rust::Notification::new();
    n.appname("RustCleanup").summary(title).body(body).urgency(urgency);
    if let Some(icon) = &icon {
        n.icon(icon);
    }
    if n.show().is_ok() {
        return;
    }
    // Root may not be let onto the user's session bus: ask notify-send,
    // running as the user, instead.
    let urgency = match kind {
        NotifyKind::Info => "low",
        NotifyKind::Warning => "normal",
        NotifyKind::Error => "critical",
    };
    let mut cmd = user_command("notify-send");
    cmd.args(["-a", "RustCleanup", "-u", urgency]);
    if let Some(icon) = &icon {
        cmd.args(["-i", icon]);
    }
    let _ = cmd
        .arg("--")
        .arg(title)
        .arg(body)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}
