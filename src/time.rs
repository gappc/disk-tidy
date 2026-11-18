//! Timestamps from docker and trash info files, and ages as short German text.

/// Unix seconds of `YYYY-MM-DD HH:MM:SS` or `YYYY-MM-DDTHH:MM:SS`, optionally with fractional
/// seconds and a ` ±HHMM` offset followed by anything (docker: `… +0200 CEST`). Without an
/// offset the time is read as UTC.
pub fn parse_timestamp(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let part = b.get(from..from + len)?;
        part.iter()
            .all(u8::is_ascii_digit)
            .then(|| std::str::from_utf8(part).ok()?.parse().ok())?
    };
    let sep = |at: usize, ok: &[u8]| b.get(at).is_some_and(|c| ok.contains(c));
    if !(sep(4, b"-") && sep(7, b"-") && sep(10, b" T") && sep(13, b":") && sep(16, b":")) {
        return None;
    }
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (h, mi, sec) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut rest = &s[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        rest = frac.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest.strip_prefix(' ').map(str::as_bytes) {
        Some([sign @ (b'+' | b'-'), digits @ ..]) if digits.len() >= 4 => {
            let o = std::str::from_utf8(&digits[..4]).ok()?;
            if !o.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let mins = o[..2].parse::<i64>().ok()? * 60 + o[2..].parse::<i64>().ok()?;
            if *sign == b'+' { mins * 60 } else { -mins * 60 }
        }
        _ => 0,
    };
    let secs = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + sec - offset;
    u64::try_from(secs).ok()
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// How long ago `then` was, seen from `now`: "heute", "vor 41 Tg.", "vor 5 Mon.", "vor 2 J.".
pub fn fmt_age(now: u64, then: u64) -> String {
    let days = now.saturating_sub(then) / 86_400;
    match days {
        0 => "heute".into(),
        1..60 => format!("vor {days} Tg."),
        60..730 => format!("vor {} Mon.", days / 30),
        _ => format!("vor {} J.", days / 365),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_docker_and_trash_formats() {
        assert_eq!(
            parse_timestamp("2026-10-08 12:00:00 +0200 CEST"),
            Some(1791453600)
        );
        assert_eq!(parse_timestamp("2026-10-08T12:00:00"), Some(1791460800));
        assert_eq!(
            parse_timestamp("2026-10-08 10:00:00.318356471 +0000 UTC"),
            Some(1791453600)
        );
        assert_eq!(
            parse_timestamp("2026-10-08 08:30:00 -0130"),
            Some(1791453600)
        );
        assert_eq!(parse_timestamp("1970-01-01T00:00:00"), Some(0));
        assert_eq!(parse_timestamp("x"), None);
        assert_eq!(parse_timestamp("2026-13-08T12:00:00"), None);
        assert_eq!(parse_timestamp("2026-10-08"), None);
    }

    #[test]
    fn fmt_age_steps() {
        let day = 86_400;
        let now = 1_000 * day;
        assert_eq!(fmt_age(now, now), "heute");
        assert_eq!(fmt_age(now, now + 5), "heute");
        assert_eq!(fmt_age(now, now - 41 * day), "vor 41 Tg.");
        assert_eq!(fmt_age(now, now - 150 * day), "vor 5 Mon.");
        assert_eq!(fmt_age(now, now - 800 * day), "vor 2 J.");
    }
}
