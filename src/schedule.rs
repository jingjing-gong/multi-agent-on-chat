use anyhow::{bail, Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Tz;
use std::str::FromStr;

pub fn next(spec: &str, timezone: &str, after: i64) -> Result<Option<i64>> {
    let tz = Tz::from_str(timezone).context("invalid IANA timezone")?;
    let (kind, value) = spec
        .split_once(' ')
        .context("schedule must be at <ISO>, every <duration>, or cron <five fields>")?;
    match kind {
        "at" => {
            let t = if let Ok(t) = DateTime::parse_from_rfc3339(value) {
                t.timestamp()
            } else {
                let n = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S")
                    .or_else(|_| chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M"))?;
                tz.from_local_datetime(&n)
                    .single()
                    .context("ambiguous or nonexistent local time; include UTC offset")?
                    .timestamp()
            };
            Ok((t > after).then_some(t))
        }
        "every" => Ok(Some(
            after
                .checked_add(duration(value)?)
                .context("duration overflow")?,
        )),
        "cron" => {
            if value.split_whitespace().count() != 5 {
                bail!("cron requires five fields");
            }
            let fields = value.split_whitespace().collect::<Vec<_>>();
            let dow = normalize_weekdays(fields[4])?;
            let expression = format!(
                "0 {} {} {} {} {} *",
                fields[0], fields[1], fields[2], fields[3], dow
            );
            let schedule =
                cron::Schedule::from_str(&expression).context("invalid cron expression")?;
            let dt = Utc
                .timestamp_opt(after, 0)
                .single()
                .context("invalid timestamp")?
                .with_timezone(&tz);
            Ok(schedule.after(&dt).next().map(|d| d.timestamp()))
        }
        _ => bail!("unsupported schedule kind"),
    }
}
// Public five-field cron uses 0/7=Sunday, 1=Monday; crate uses 1=Sunday.
fn normalize_weekdays(field: &str) -> Result<String> {
    let mut days = std::collections::BTreeSet::new();
    let parse = |s: &str| -> Result<u32> {
        let n = match s.to_ascii_lowercase().as_str() {
            "sun" => 0,
            "mon" => 1,
            "tue" => 2,
            "wed" => 3,
            "thu" => 4,
            "fri" => 5,
            "sat" => 6,
            _ => s.parse::<u32>()?,
        };
        if n > 7 {
            bail!("invalid weekday");
        }
        Ok(n)
    };
    for piece in field.split(',') {
        let (range, step) = if let Some((r, s)) = piece.split_once('/') {
            (r, s.parse::<u32>()?)
        } else {
            (piece, 1)
        };
        if step == 0 {
            bail!("weekday step must be positive");
        }
        let (lo, hi) = if range == "*" {
            (0, 6)
        } else if let Some((a, b)) = range.split_once('-') {
            (parse(a)?, parse(b)?)
        } else {
            let n = parse(range)?;
            (n, if piece.contains('/') { 7 } else { n })
        };
        if lo > hi {
            bail!("weekday range must be ascending");
        }
        for n in (lo..=hi).step_by(step as usize) {
            days.insert(n % 7 + 1);
        }
    }
    Ok(days
        .into_iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(","))
}

pub fn duration(s: &str) -> Result<i64> {
    let mut digits = String::new();
    let mut total = 0i64;
    for c in s.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let factor = match c {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            'd' => 86400,
            _ => bail!("invalid duration unit"),
        };
        let n: i64 = digits.parse().context("missing duration number")?;
        digits.clear();
        total = total
            .checked_add(n.checked_mul(factor).context("duration overflow")?)
            .context("duration overflow")?;
    }
    if !digits.is_empty() || total < 1 {
        bail!("duration must be positive and include a unit");
    }
    Ok(total)
}
pub fn advance(spec: &str, tz: &str, scheduled: i64, at: i64) -> Result<Option<i64>> {
    if spec.starts_with("at ") {
        return Ok(None);
    }
    if let Some(d) = spec.strip_prefix("every ") {
        let step = duration(d)?;
        let n = ((at - scheduled).max(0) / step) + 1;
        return Ok(Some(
            scheduled
                .checked_add(n.checked_mul(step).context("overflow")?)
                .context("overflow")?,
        ));
    }
    next(spec, tz, at)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn weekdays_use_public_cron_numbering() {
        let sunday = DateTime::parse_from_rfc3339("2026-09-27T00:00:00+08:00")
            .unwrap()
            .timestamp();
        let monday = DateTime::parse_from_rfc3339("2026-09-28T09:00:00+08:00")
            .unwrap()
            .timestamp();
        assert_eq!(
            next("cron 0 9 * * 1-5", "Asia/Shanghai", sunday).unwrap(),
            Some(monday)
        );
        assert_eq!(normalize_weekdays("0,7").unwrap(), "1");
        assert_eq!(normalize_weekdays("MON-FRI").unwrap(), "2,3,4,5,6");
    }
    #[test]
    fn intervals_do_not_drift() {
        assert_eq!(
            advance("every 10m", "Asia/Shanghai", 100, 1550).unwrap(),
            Some(1900)
        );
    }
    #[test]
    fn local_timezone() {
        assert_eq!(
            next("at 2026-09-28T09:00", "Asia/Shanghai", 0).unwrap(),
            Some(
                DateTime::parse_from_rfc3339("2026-09-28T01:00:00Z")
                    .unwrap()
                    .timestamp()
            )
        );
    }
    #[test]
    fn reject_malformed() {
        assert!(duration("1h2").is_err());
        assert!(next("cron 0 1 * *", "UTC", 0).is_err());
    }
}
