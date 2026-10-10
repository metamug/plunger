//! Showing the time a request was sent: in the user's own time zone when the system will tell us
//! (it does not always, on Linux), otherwise in UTC and labelled so.

use time::format_description::{self, well_known::Rfc3339};
use time::{OffsetDateTime, UtcOffset};

/// `ts` (RFC 3339) in local time when known, with whether it is local.
fn convert(ts: &str) -> Option<(OffsetDateTime, bool)> {
    let parsed = OffsetDateTime::parse(ts, &Rfc3339).ok()?;
    match UtcOffset::current_local_offset() {
        Ok(offset) => Some((parsed.to_offset(offset), true)),
        Err(_) => Some((parsed, false)),
    }
}

fn format(dt: OffsetDateTime, pattern: &str) -> String {
    format_description::parse_borrowed::<2>(pattern).ok().and_then(|f| dt.format(&f).ok()).unwrap_or_default()
}

/// "06:11:03", the time of day only (local when known, else UTC).
pub fn clock(ts: &str) -> String {
    convert(ts).map(|(dt, _)| format(dt, "[hour]:[minute]:[second]")).unwrap_or_default()
}

/// How long ago `ts` was, for a list: "just now", "5 min ago", "3 h ago", "2 d ago". A timestamp that
/// cannot be read gives an empty string.
pub fn ago(ts: &str) -> String {
    ago_from(OffsetDateTime::now_utc(), ts)
}

fn ago_from(now: OffsetDateTime, ts: &str) -> String {
    let Ok(then) = OffsetDateTime::parse(ts, &Rfc3339) else { return String::new() };
    let seconds = (now - then).whole_seconds();
    match seconds {
        s if s < 45 => "just now".to_string(),
        s if s < 3600 => format!("{} min ago", (s + 30) / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s => format!("{} d ago", s / 86_400),
    }
}

/// "2026-10-09 06:11:03", with " UTC" added when the local zone is not known. Anything that is not
/// an RFC 3339 timestamp is returned as it is.
pub fn full(ts: &str) -> String {
    match convert(ts) {
        Some((dt, true)) => format(dt, "[year]-[month]-[day] [hour]:[minute]:[second]"),
        Some((dt, false)) => format!("{} UTC", format(dt, "[year]-[month]-[day] [hour]:[minute]:[second]")),
        None => ts.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timestamp_is_shown_without_fractions_or_the_t() {
        let shown = full("2026-10-09T06:11:03.1234567Z");
        assert!(shown.contains(' ') && !shown.contains('T') && !shown.contains('.'), "{shown}");
        let c = clock("2026-10-09T06:11:03Z");
        assert_eq!(c.len(), 8, "{c}");
    }

    #[test]
    fn a_list_says_how_long_ago_in_plain_words() {
        let now = OffsetDateTime::parse("2026-10-10T12:00:00Z", &Rfc3339).unwrap();
        assert_eq!(ago_from(now, "2026-10-10T11:59:50Z"), "just now");
        assert_eq!(ago_from(now, "2026-10-10T12:00:30Z"), "just now", "a clock a little ahead is not negative minutes");
        assert_eq!(ago_from(now, "2026-10-10T11:55:00Z"), "5 min ago");
        assert_eq!(ago_from(now, "2026-10-10T09:00:00Z"), "3 h ago");
        assert_eq!(ago_from(now, "2026-10-08T12:00:00Z"), "2 d ago");
        assert_eq!(ago_from(now, "nonsense"), "");
    }

    #[test]
    fn something_that_is_not_a_timestamp_is_left_alone() {
        assert_eq!(full("yesterday"), "yesterday");
        assert_eq!(clock("yesterday"), "");
        assert_eq!(full(""), "");
    }
}
