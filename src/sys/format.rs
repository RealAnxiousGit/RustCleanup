//! Formatting helpers: byte sizes and timestamps.

use crate::i18n::{Lang, language, tr, trf};

/// Formats a byte count as a short human-readable string.
pub fn fmt_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Current local time as `HH:MM:SS`, for log timestamps.
#[cfg(windows)]
pub fn local_time_hms() -> String {
    // SAFETY: GetLocalTime has no failure mode.
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond)
}

/// Current local time as `HH:MM:SS`, for log timestamps.
#[cfg(target_os = "linux")]
pub fn local_time_hms() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let t = local_tm(now);
    format!("{:02}:{:02}:{:02}", t.tm_hour, t.tm_min, t.tm_sec)
}

/// Unix seconds to local broken-down time.
#[cfg(target_os = "linux")]
fn local_tm(secs: i64) -> libc::tm {
    // SAFETY: localtime_r fills `tm`; an all-zero tm is a valid fallback.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        let t = secs as libc::time_t;
        unsafe extern "C" {
            fn tzset();
        }
        tzset();
        libc::localtime_r(&t, &mut tm);
        tm
    }
}

/// Days from 1970-01-01 to a civil date (inverse of `civil_from_days`).
#[cfg(target_os = "linux")]
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// UTC timestamp `days` from now in the `YYYY-MM-DDTHH:MM:SSZ` form that
/// the Windows Update pause settings use.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn utc_iso_in_days(days: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let secs = now + days * 86_400;
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let rem = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

/// `2026-12-06T10:00:00Z` -> `6 Dec 2026`, for showing the update pause date.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn friendly_date(iso: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut parts = iso
        .get(..10)
        .unwrap_or("")
        .split('-')
        .map(|p| p.parse::<usize>().ok());
    match (
        parts.next().flatten(),
        parts.next().flatten(),
        parts.next().flatten(),
    ) {
        (Some(y), Some(m @ 1..=12), Some(d)) => format!("{d} {} {y}", MONTHS[m - 1]),
        _ => iso.to_string(),
    }
}

/// `2026-11-02T18:30:00Z` -> `November 2nd, 2026`, in the local time zone,
/// for the date a release was published.
pub fn long_date(iso: &str) -> Option<String> {
    let num = |range: std::ops::Range<usize>| iso.get(range)?.parse::<u16>().ok();
    let utc = [
        num(0..4)?,
        num(5..7).filter(|m| (1..=12).contains(m))?,
        num(8..10).filter(|d| (1..=31).contains(d))?,
        num(11..13).unwrap_or(0),
        num(14..16).unwrap_or(0),
        num(17..19).unwrap_or(0),
    ];
    let (year, month, day) = to_local_date(utc);
    let day = match language() {
        Lang::En => ordinal(u32::from(day)),
        _ => day.to_string(),
    };
    Some(trf("{0} {1}, {2}", &[&month_name(month), &day, &year]))
}

/// A UTC [year, month, day, hour, minute, second] to the local (year, month, day).
#[cfg(windows)]
fn to_local_date(utc: [u16; 6]) -> (u16, u16, u16) {
    use windows::Win32::Foundation::SYSTEMTIME;
    use windows::Win32::System::Time::SystemTimeToTzSpecificLocalTime;

    let utc = SYSTEMTIME {
        wYear: utc[0],
        wMonth: utc[1],
        wDay: utc[2],
        wHour: utc[3],
        wMinute: utc[4],
        wSecond: utc[5],
        ..Default::default()
    };
    let mut local = utc;
    // SAFETY: both pointers are valid SYSTEMTIMEs; on failure the UTC date stays.
    if unsafe { SystemTimeToTzSpecificLocalTime(None, &utc, &mut local) }.is_err() {
        local = utc;
    }
    (local.wYear, local.wMonth, local.wDay)
}

/// A UTC [year, month, day, hour, minute, second] to the local (year, month, day).
#[cfg(target_os = "linux")]
fn to_local_date(utc: [u16; 6]) -> (u16, u16, u16) {
    let days = days_from_civil(i64::from(utc[0]), u32::from(utc[1]), u32::from(utc[2]));
    let secs = days * 86_400 + i64::from(utc[3]) * 3600 + i64::from(utc[4]) * 60 + i64::from(utc[5]);
    let tm = local_tm(secs);
    if tm.tm_mday == 0 {
        return (utc[0], utc[1], utc[2]);
    }
    ((tm.tm_year + 1900) as u16, (tm.tm_mon + 1) as u16, tm.tm_mday as u16)
}

fn month_name(month: u16) -> &'static str {
    match month {
        1 => tr("January"),
        2 => tr("February"),
        3 => tr("March"),
        4 => tr("April"),
        5 => tr("May"),
        6 => tr("June"),
        7 => tr("July"),
        8 => tr("August"),
        9 => tr("September"),
        10 => tr("October"),
        11 => tr("November"),
        _ => tr("December"),
    }
}

/// `2` -> `2nd`, `11` -> `11th`, `23` -> `23rd`.
fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// Days since 1970-01-01 to a (year, month, day) civil date
/// (Howard Hinnant's algorithm, valid for the whole proleptic Gregorian range).
#[cfg_attr(not(windows), allow(dead_code))]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_758), (2026, 11, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        #[cfg(target_os = "linux")]
        for d in [0, 11_016, 20_758, -1, 50_000] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
    }

    #[test]
    fn bytes_format() {
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(1536), "1.5 KB");
        assert_eq!(fmt_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn dates_for_people() {
        assert_eq!(friendly_date("2026-12-06T10:00:00Z"), "6 Dec 2026");
        assert_eq!(friendly_date("garbage"), "garbage");
        // Midday UTC is the same day in every time zone up to 12 hours away.
        assert_eq!(
            long_date("2026-11-02T12:00:00Z").as_deref(),
            Some("November 2nd, 2026")
        );
        assert_eq!(long_date("garbage"), None);
        let ordinals = [1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 101, 111].map(ordinal);
        assert_eq!(
            ordinals.join(" "),
            "1st 2nd 3rd 4th 11th 12th 13th 21st 22nd 23rd 101st 111th"
        );
    }
}
