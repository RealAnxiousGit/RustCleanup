//! Interface languages.
//!
//! English text is the key: `tr("Clean RAM")` looks the string up in the
//! current language's file under `lang/` and falls back to the English when
//! a translation is missing. `trf` does the same for text with `{0}`, `{1}`
//! placeholders, so translations can reorder them. `n_` only marks a string
//! for translation (for tables of labels) and is translated where it is shown.
//!
//! The `every_string_is_translated` test reads the source, collects every
//! marked string and fails if any language file is missing one.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(windows)]
use windows::Win32::Globalization::GetUserDefaultUILanguage;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    En,
    Es,
    Pt,
    Fr,
    De,
    It,
    Nl,
    Pl,
    Tr,
    Ru,
    Ja,
    Zh,
}

impl Lang {
    pub const ALL: [Lang; 12] = [
        Lang::En,
        Lang::Es,
        Lang::Pt,
        Lang::Fr,
        Lang::De,
        Lang::It,
        Lang::Nl,
        Lang::Pl,
        Lang::Tr,
        Lang::Ru,
        Lang::Ja,
        Lang::Zh,
    ];

    /// ISO 639-1 code, also the file name under `lang/` and the saved setting.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Es => "es",
            Lang::Pt => "pt",
            Lang::Fr => "fr",
            Lang::De => "de",
            Lang::It => "it",
            Lang::Nl => "nl",
            Lang::Pl => "pl",
            Lang::Tr => "tr",
            Lang::Ru => "ru",
            Lang::Ja => "ja",
            Lang::Zh => "zh",
        }
    }

    /// The language's name in that language, as the picker shows it.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Es => "Español",
            Lang::Pt => "Português",
            Lang::Fr => "Français",
            Lang::De => "Deutsch",
            Lang::It => "Italiano",
            Lang::Nl => "Nederlands",
            Lang::Pl => "Polski",
            Lang::Tr => "Türkçe",
            Lang::Ru => "Русский",
            Lang::Ja => "日本語",
            Lang::Zh => "简体中文",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.code() == code)
    }

    /// The Windows display language, if RustCleanup has it; otherwise English.
    #[cfg(target_os = "linux")]
    pub fn system_default() -> Lang {
        // The first of these that is set wins, as in gettext.
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .find_map(|v| std::env::var(v).ok().filter(|l| !l.is_empty()))
            .unwrap_or_default();
        match locale.get(..2).unwrap_or("") {
            "es" => Lang::Es,
            "pt" => Lang::Pt,
            "fr" => Lang::Fr,
            "de" => Lang::De,
            "it" => Lang::It,
            "nl" => Lang::Nl,
            "pl" => Lang::Pl,
            "tr" => Lang::Tr,
            "ru" => Lang::Ru,
            "ja" => Lang::Ja,
            "zh" => Lang::Zh,
            _ => Lang::En,
        }
    }

    #[cfg(windows)]
    pub fn system_default() -> Lang {
        // SAFETY: no parameters, no failure mode.
        let primary = unsafe { GetUserDefaultUILanguage() } & 0x3FF;
        match primary {
            0x0A => Lang::Es,
            0x16 => Lang::Pt,
            0x0C => Lang::Fr,
            0x07 => Lang::De,
            0x10 => Lang::It,
            0x13 => Lang::Nl,
            0x15 => Lang::Pl,
            0x1F => Lang::Tr,
            0x19 => Lang::Ru,
            0x11 => Lang::Ja,
            0x04 => Lang::Zh,
            _ => Lang::En,
        }
    }

    fn source(self) -> &'static str {
        match self {
            Lang::En => "",
            Lang::Es => include_str!("../lang/es.txt"),
            Lang::Pt => include_str!("../lang/pt.txt"),
            Lang::Fr => include_str!("../lang/fr.txt"),
            Lang::De => include_str!("../lang/de.txt"),
            Lang::It => include_str!("../lang/it.txt"),
            Lang::Nl => include_str!("../lang/nl.txt"),
            Lang::Pl => include_str!("../lang/pl.txt"),
            Lang::Tr => include_str!("../lang/tr.txt"),
            Lang::Ru => include_str!("../lang/ru.txt"),
            Lang::Ja => include_str!("../lang/ja.txt"),
            Lang::Zh => include_str!("../lang/zh.txt"),
        }
    }

    fn index(self) -> usize {
        Lang::ALL.iter().position(|&l| l == self).unwrap_or(0)
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_language(lang: Lang) {
    CURRENT.store(lang.index() as u8, Ordering::Relaxed);
}

pub fn language() -> Lang {
    Lang::ALL
        .get(CURRENT.load(Ordering::Relaxed) as usize)
        .copied()
        .unwrap_or(Lang::En)
}

type Table = HashMap<&'static str, &'static str>;

/// Parses a language file: entries are an English line followed by its
/// translation, separated by blank lines. Lines starting with `#` are comments.
fn parse(source: &'static str) -> Table {
    let mut table = Table::new();
    let mut lines = source
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'));
    while let Some(key) = lines.next() {
        if key.is_empty() {
            continue;
        }
        if let Some(value) = lines.next().filter(|v| !v.is_empty()) {
            table.insert(key, value);
        }
    }
    table
}

fn table(lang: Lang) -> &'static Table {
    static TABLES: [OnceLock<Table>; 12] = [const { OnceLock::new() }; 12];
    TABLES[lang.index()].get_or_init(|| parse(lang.source()))
}

/// Translates `key` into the current language.
pub fn tr(key: &'static str) -> &'static str {
    match language() {
        Lang::En => key,
        lang => table(lang).get(key).copied().unwrap_or(key),
    }
}

/// Translates `key` and fills in `{0}`, `{1}`, ... with `args`.
pub fn trf(key: &'static str, args: &[&dyn Display]) -> String {
    let mut text = tr(key).to_string();
    for (i, arg) in args.iter().enumerate() {
        text = text.replace(&format!("{{{i}}}"), &arg.to_string());
    }
    text
}

/// Marks a string for translation without translating it yet.
pub const fn n_(key: &'static str) -> &'static str {
    key
}

/// Picks the Windows or the Linux wording of a string. Both are marked with
/// `n_` at the call site so the translation test sees them:
/// `tr(os(n_(WINDOWS_TEXT), n_(LINUX_TEXT)))`, with string literals in place
/// of the two names.
#[allow(unused_variables)]
pub const fn os(windows: &'static str, linux: &'static str) -> &'static str {
    #[cfg(windows)]
    return windows;
    #[cfg(not(windows))]
    return linux;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `tr("...")`, `trf("...")` and `n_("...")` literal in the source.
    fn marked_strings() -> Vec<String> {
        fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let src = std::fs::read_to_string(&path).unwrap();
                    // Built at run time so this loop doesn't match itself.
                    for marker in ["tr", "trf", "n_"].map(|name| format!("{name}(")) {
                        let mut rest = src.as_str();
                        while let Some(at) = rest.find(&marker) {
                            // Skip identifiers that merely end in the marker (e.g. `attr(`).
                            let before = rest[..at].chars().last();
                            rest = &rest[at + marker.len()..];
                            if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                                continue;
                            }
                            // The literal may start on the next line.
                            let Some(literal) = rest.trim_start().strip_prefix('"') else {
                                continue;
                            };
                            rest = literal;
                            let mut key = String::new();
                            let mut chars = rest.chars();
                            while let Some(c) = chars.next() {
                                match c {
                                    '"' => break,
                                    '\\' => match chars.next() {
                                        Some('n') => key.push('\n'),
                                        Some(other) => key.push(other),
                                        None => {}
                                    },
                                    c => key.push(c),
                                }
                            }
                            out.push(key);
                        }
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut out,
        );
        out.sort();
        out.dedup();
        out.retain(|k| k != "...");
        out
    }

    fn placeholders(s: &str) -> Vec<String> {
        let mut found: Vec<String> = (0..10)
            .map(|i| format!("{{{i}}}"))
            .filter(|p| s.contains(p.as_str()))
            .collect();
        found.sort();
        found
    }

    #[test]
    fn every_string_is_translated() {
        let keys = marked_strings();
        assert!(keys.len() > 100, "found only {} strings", keys.len());
        let mut problems = Vec::new();
        for lang in Lang::ALL.into_iter().skip(1) {
            let table = table(lang);
            for key in &keys {
                match table.get(key.as_str()) {
                    None => problems.push(format!("{}: missing {key:?}", lang.code())),
                    Some(value) if placeholders(value) != placeholders(key) => {
                        problems.push(format!("{}: placeholders differ in {key:?}", lang.code()))
                    }
                    Some(value) if value.contains('\u{2014}') => {
                        problems.push(format!("{}: em dash in {key:?}", lang.code()))
                    }
                    _ => {}
                }
            }
        }
        assert!(
            problems.is_empty(),
            "{} problems:\n{}",
            problems.len(),
            problems.join("\n")
        );
    }

    #[test]
    fn falls_back_to_english() {
        // Keys go through variables so the source scan above doesn't pick them up.
        let (missing, template) = ("no such string, surely", "{0} of {1}");
        assert_eq!(*table(Lang::Es).get(missing).unwrap_or(&missing), missing);
        assert_eq!(trf(template, &[&1, &2]), "1 of 2");
    }
}
