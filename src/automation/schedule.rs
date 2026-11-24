use chrono::{Datelike, NaiveTime, Timelike};

use crate::automation::schema::SunEvent;

/// Parse a strict wall-clock time string (`HH:MM:SS` or `HH:MM`).
pub fn parse_wall_time(input: &str) -> Result<NaiveTime, String> {
    let trimmed = input.trim();
    if let Ok(t) = NaiveTime::parse_from_str(trimmed, "%H:%M:%S") {
        return Ok(t);
    }
    if let Ok(t) = NaiveTime::parse_from_str(trimmed, "%H:%M") {
        return Ok(t);
    }
    Err(format!(
        "invalid time '{input}' (expected HH:MM:SS or HH:MM)"
    ))
}

/// Evaluate whether `now` falls inside an optional `[after, before)` wall-clock window.
/// Supports midnight wrap-around (e.g., `after = 22:00:00`, `before = 06:30:00`).
pub fn time_in_window(now: NaiveTime, after: Option<NaiveTime>, before: Option<NaiveTime>) -> bool {
    match (after, before) {
        (Some(a), Some(b)) => {
            if a <= b {
                now >= a && now < b
            } else {
                // Midnight wrap-around: [a, 24:00) U [00:00, b)
                now >= a || now < b
            }
        }
        (Some(a), None) => now >= a,
        (None, Some(b)) => now < b,
        (None, None) => true,
    }
}

/// Parsed 5-field (`min hour dom month dow`) or 6-field (`sec min hour dom month dow`) cron schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSchedule {
    seconds: Vec<u32>,
    minutes: Vec<u32>,
    hours: Vec<u32>,
    days_of_month: Vec<u32>,
    months: Vec<u32>,
    days_of_week: Vec<u32>, // 0 = Sun, 1 = Mon, ..., 6 = Sat
}

impl CronSchedule {
    pub fn parse(expr: &str) -> Result<Self, String> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        let (sec_str, min_str, hour_str, dom_str, mon_str, dow_str) = match parts.len() {
            5 => ("0", parts[0], parts[1], parts[2], parts[3], parts[4]),
            6 => (parts[0], parts[1], parts[2], parts[3], parts[4], parts[5]),
            n => {
                return Err(format!(
                    "cron expression '{expr}' has {n} fields (expected 5 or 6)"
                ));
            }
        };

        let seconds = parse_cron_field(sec_str, 0, 59, false)?;
        let minutes = parse_cron_field(min_str, 0, 59, false)?;
        let hours = parse_cron_field(hour_str, 0, 23, false)?;
        let days_of_month = parse_cron_field(dom_str, 1, 31, false)?;
        let months = parse_cron_field(mon_str, 1, 12, false)?;
        let days_of_week = parse_cron_field(dow_str, 0, 7, true)?;

        Ok(Self {
            seconds,
            minutes,
            hours,
            days_of_month,
            months,
            days_of_week,
        })
    }

    /// Return `true` if `dt` (truncated to second precision) matches this cron schedule.
    pub fn matches<Tz: chrono::TimeZone>(&self, dt: &chrono::DateTime<Tz>) -> bool {
        let dow = dt.weekday().num_days_from_sunday(); // 0 = Sun .. 6 = Sat
        self.seconds.contains(&dt.second())
            && self.minutes.contains(&dt.minute())
            && self.hours.contains(&dt.hour())
            && self.days_of_month.contains(&dt.day())
            && self.months.contains(&dt.month())
            && self.days_of_week.contains(&dow)
    }
}

fn parse_cron_field(
    field: &str,
    min: u32,
    max: u32,
    wrap_sunday_7: bool,
) -> Result<Vec<u32>, String> {
    let mut values = Vec::new();
    for segment in field.split(',') {
        let (range_part, step) = match segment.split_once('/') {
            Some((lhs, rhs)) => {
                let s: u32 = rhs
                    .parse()
                    .map_err(|_| format!("invalid cron step '{rhs}' in '{field}'"))?;
                if s == 0 {
                    return Err(format!("cron step must be >= 1 in '{field}'"));
                }
                (lhs, s)
            }
            None => (segment, 1),
        };

        let (start, end) = if range_part == "*" {
            (min, if wrap_sunday_7 { 6 } else { max })
        } else if let Some((a, b)) = range_part.split_once('-') {
            let s = parse_bound(a, min, max, field)?;
            let e = parse_bound(b, min, max, field)?;
            if s > e {
                return Err(format!("cron range start > end in '{field}'"));
            }
            (s, e)
        } else {
            let single = parse_bound(range_part, min, max, field)?;
            if segment.contains('/') {
                (single, if wrap_sunday_7 { 6 } else { max })
            } else {
                (single, single)
            }
        };

        let mut v = start;
        while v <= end {
            let normalized = if wrap_sunday_7 && v == 7 { 0 } else { v };
            if !values.contains(&normalized) {
                values.push(normalized);
            }
            v = v.saturating_add(step);
            if v == start {
                break;
            }
        }
    }
    if values.is_empty() {
        return Err(format!("empty cron field '{field}'"));
    }
    values.sort_unstable();
    Ok(values)
}

fn parse_bound(token: &str, min: u32, max: u32, field: &str) -> Result<u32, String> {
    let val: u32 = token
        .parse()
        .map_err(|_| format!("invalid cron value '{token}' in '{field}'"))?;
    if val < min || val > max {
        return Err(format!(
            "cron value {val} out of range [{min}, {max}] in '{field}'"
        ));
    }
    Ok(val)
}

/// Compute approximate UTC sunrise or sunset `NaiveTime` for `(latitude, longitude)` on `date`
/// using the standard NOAA solar zenith algorithm (zenith = 90.833°).
///
/// Returns `None` during polar day / polar night when the sun does not cross the horizon.
pub fn compute_sun_time_utc(
    date: chrono::NaiveDate,
    latitude: f64,
    longitude: f64,
    event: SunEvent,
) -> Option<NaiveTime> {
    let n = f64::from(date.ordinal());
    let lng_hour = longitude / 15.0;
    let t = match event {
        SunEvent::Sunrise => n + ((6.0 - lng_hour) / 24.0),
        SunEvent::Sunset => n + ((18.0 - lng_hour) / 24.0),
    };

    let m = (0.9856 * t) - 3.289;
    let mut l =
        m + (1.916 * m.to_radians().sin()) + (0.020 * (2.0 * m).to_radians().sin()) + 282.634;
    l = l.rem_euclid(360.0);

    let mut ra = (0.91764 * l.to_radians().tan()).atan().to_degrees();
    ra = ra.rem_euclid(360.0);
    let l_quadrant = (l / 90.0).floor() * 90.0;
    let ra_quadrant = (ra / 90.0).floor() * 90.0;
    ra = (ra + (l_quadrant - ra_quadrant)) / 15.0;

    let sin_dec = 0.39782 * l.to_radians().sin();
    let cos_dec = sin_dec.asin().cos();

    let zenith = 90.833_f64;
    let cos_h = (zenith.to_radians().cos() - (sin_dec * latitude.to_radians().sin()))
        / (cos_dec * latitude.to_radians().cos());
    if !(-1.0..=1.0).contains(&cos_h) {
        return None;
    }

    let h = match event {
        SunEvent::Sunrise => (360.0 - cos_h.acos().to_degrees()) / 15.0,
        SunEvent::Sunset => cos_h.acos().to_degrees() / 15.0,
    };

    let local_mean_time = h + ra - (0.06571 * t) - 6.622;
    let ut = (local_mean_time - lng_hour).rem_euclid(24.0);
    let total_secs = (ut * 3600.0).round() as u32 % 86_400;
    NaiveTime::from_num_seconds_from_midnight_opt(total_secs, 0)
}
