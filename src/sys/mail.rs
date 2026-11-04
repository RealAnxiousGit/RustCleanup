//! Which app opens `mailto:` links, so the Feedback window can say what
//! went wrong instead of claiming an email draft opened when none did.
//!
//! Opening a link always "succeeds" from RustCleanup's side (Explorer or
//! xdg-open take it and return), so the handler is looked up first. Each
//! system's `shell::mail_handler` fills this in.

/// The app set to open email links.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MailHandler {
    /// Nothing is set up to open email links.
    Missing,
    /// An app is set up. `browser` is true for web browsers, which often
    /// show nothing or an empty tab for an email link.
    App { name: String, browser: bool },
    /// The lookup itself failed; the reason is kept for the log.
    Unknown(String),
}

impl MailHandler {
    /// Builds the result from an app's name and its program or desktop
    /// file, guessing from both whether it is a web browser.
    pub fn app(name: impl Into<String>, program: &str) -> Self {
        let name = name.into();
        let browser = is_browser(&name) || is_browser(program);
        MailHandler::App { name, browser }
    }
}

/// True for the names, programs and desktop files of common web browsers.
pub fn is_browser(id: &str) -> bool {
    const BROWSERS: [&str; 18] = [
        "firefox", "chrome", "chromium", "msedge", "microsoft edge", "brave", "opera",
        "vivaldi", "iexplore", "internet explorer", "librewolf", "waterfox", "floorp",
        "epiphany", "falkon", "thorium", "zen", "web browser",
    ];
    let id = id.to_lowercase();
    // Only the file name counts: a folder called "Zen" is not a browser.
    let file = id.rsplit(['\\', '/']).next().unwrap_or(&id);
    BROWSERS.iter().any(|b| file.contains(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browsers_are_spotted() {
        assert!(is_browser(r"C:\Program Files\Mozilla Firefox\firefox.exe"));
        assert!(is_browser("Firefox"));
        assert!(is_browser("firefox_firefox.desktop"));
        assert!(is_browser("Microsoft Edge"));
        assert!(is_browser("google-chrome.desktop"));
        assert!(!is_browser(r"C:\Program Files\Microsoft Office\root\Office16\OUTLOOK.EXE"));
        assert!(!is_browser("Thunderbird"));
        assert!(!is_browser("org.gnome.Evolution.desktop"));
        assert!(!is_browser("Mail"));
        assert!(!is_browser(r"C:\Zen\mailapp.exe"));
    }

    /// Prints this PC's email app; run with `--ignored --nocapture`.
    #[test]
    #[ignore = "reads this desktop's default apps"]
    fn this_desktops_mail_handler() {
        println!("mail handler: {:?}", crate::sys::shell::mail_handler());
        println!("running now: {:?}", crate::sys::shell::mail_app_running());
    }

    #[test]
    fn app_uses_name_and_program() {
        assert_eq!(
            MailHandler::app("Firefox", r"C:\x\firefox.exe"),
            MailHandler::App { name: "Firefox".into(), browser: true }
        );
        assert_eq!(
            MailHandler::app("Outlook", r"C:\x\olk.exe"),
            MailHandler::App { name: "Outlook".into(), browser: false }
        );
    }
}
