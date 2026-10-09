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
    fn something_that_is_not_a_timestamp_is_left_alone() {
        assert_eq!(full("yesterday"), "yesterday");
        assert_eq!(clock("yesterday"), "");
        assert_eq!(full(""), "");
    }
}
