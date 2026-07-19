use chrono::{
    DateTime, Datelike, Duration, FixedOffset, LocalResult, NaiveDate, TimeZone, Timelike, Utc,
};
use chrono_tz::Tz;
use std::fmt;

const NANOS_PER_SECOND: i128 = 1_000_000_000;

#[derive(Debug, Clone)]
pub(crate) struct TimeContext {
    name: String,
    timezone: Tz,
    year: Option<i32>,
    period_start_nanos: Option<i128>,
    period_end_nanos: Option<i128>,
}

#[derive(Debug, Clone)]
pub(crate) struct TimeContextError {
    code: &'static str,
    message: String,
    remediation: &'static str,
}

impl TimeContextError {
    pub(crate) fn code(&self) -> &'static str {
        self.code
    }

    pub(crate) fn remediation(&self) -> &'static str {
        self.remediation
    }
}

impl fmt::Display for TimeContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TimeContextError {}

impl TimeContext {
    pub(crate) fn new(name: &str, year: Option<i32>) -> Result<Self, TimeContextError> {
        let timezone = name.parse::<Tz>().map_err(|_| TimeContextError {
            code: "E_TIMEZONE_INVALID",
            message: "the selected timezone is not a recognized IANA timezone".to_string(),
            remediation: "Use an IANA name such as UTC, Europe/Berlin, or America/New_York.",
        })?;
        let (period_start_nanos, period_end_nanos) = if let Some(year) = year {
            let start = local_date_start_nanos(
                timezone,
                NaiveDate::from_ymd_opt(year, 1, 1).ok_or_else(|| TimeContextError {
                    code: "E_PERIOD_INVALID",
                    message: "the selected year is outside the supported calendar range"
                        .to_string(),
                    remediation: "Choose a four-digit year supported by the IANA calendar.",
                })?,
            )?;
            let end_year = year.checked_add(1).ok_or_else(|| TimeContextError {
                code: "E_PERIOD_INVALID",
                message: "the selected year cannot form a bounded reporting period".to_string(),
                remediation: "Choose a four-digit year supported by the IANA calendar.",
            })?;
            let end = local_date_start_nanos(
                timezone,
                NaiveDate::from_ymd_opt(end_year, 1, 1).ok_or_else(|| TimeContextError {
                    code: "E_PERIOD_INVALID",
                    message: "the selected year cannot form a bounded reporting period".to_string(),
                    remediation: "Choose a four-digit year supported by the IANA calendar.",
                })?,
            )?;
            (Some(start), Some(end))
        } else {
            (None, None)
        };

        Ok(Self {
            name: timezone.name().to_string(),
            timezone,
            year,
            period_start_nanos,
            period_end_nanos,
        })
    }

    #[allow(dead_code)] // Used by the binary copy of the shared ingestion module.
    pub(crate) fn resolve_default(year: Option<i32>) -> (Self, bool) {
        let host = iana_time_zone::get_timezone().ok();
        Self::resolve_default_from(host.as_deref(), year)
    }

    fn resolve_default_from(host: Option<&str>, year: Option<i32>) -> (Self, bool) {
        if let Some(name) = host {
            if let Ok(context) = Self::new(name, year) {
                return (context, false);
            }
        }
        (
            Self::new("UTC", year).expect("UTC is a valid IANA timezone"),
            true,
        )
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn database_version(&self) -> &'static str {
        chrono_tz::IANA_TZDB_VERSION
    }

    pub(crate) fn year(&self) -> Option<i32> {
        self.year
    }

    #[allow(dead_code)] // Used by the binary copy of the shared ingestion module.
    pub(crate) fn current_year(&self) -> i32 {
        Utc::now().with_timezone(&self.timezone).year()
    }

    pub(crate) fn contains_fixed(&self, instant: DateTime<FixedOffset>) -> bool {
        instant
            .with_timezone(&Utc)
            .timestamp_nanos_opt()
            .is_some_and(|nanos| self.contains_epoch(i128::from(nanos)))
    }

    pub(crate) fn contains_epoch(&self, epoch_nanos: i128) -> bool {
        match (self.period_start_nanos, self.period_end_nanos) {
            (Some(start), Some(end)) => epoch_nanos >= start && epoch_nanos < end,
            _ => true,
        }
    }

    pub(crate) fn period_bounds(&self) -> Option<(i128, i128)> {
        Some((self.period_start_nanos?, self.period_end_nanos?))
    }

    pub(crate) fn clip_interval(&self, start: i128, end: i128) -> Option<(i128, i128)> {
        let (mut start, mut end) = (start, end);
        if let Some(period_start) = self.period_start_nanos {
            start = start.max(period_start);
        }
        if let Some(period_end) = self.period_end_nanos {
            end = end.min(period_end);
        }
        (end > start).then_some((start, end))
    }

    pub(crate) fn date_key_epoch(&self, epoch_nanos: i128) -> Option<String> {
        self.local_datetime(epoch_nanos)
            .map(|value| value.format("%Y-%m-%d").to_string())
    }

    pub(crate) fn local_date_epoch(&self, epoch_nanos: i128) -> Option<NaiveDate> {
        self.local_datetime(epoch_nanos)
            .map(|value| value.date_naive())
    }

    pub(crate) fn local_date_start_epoch(&self, date: NaiveDate) -> Result<i128, TimeContextError> {
        local_date_start_nanos(self.timezone, date)
    }

    #[allow(dead_code)]
    pub(crate) fn date_key_timestamp(&self, timestamp: &str) -> Option<String> {
        ccwrapped::parse_timestamp(timestamp).and_then(|instant| {
            instant
                .with_timezone(&Utc)
                .timestamp_nanos_opt()
                .and_then(|nanos| self.date_key_epoch(i128::from(nanos)))
        })
    }

    pub(crate) fn hour_epoch(&self, epoch_nanos: i128) -> Option<u8> {
        self.local_datetime(epoch_nanos)
            .map(|value| value.hour() as u8)
    }

    #[allow(dead_code)]
    pub(crate) fn weekday_epoch(&self, epoch_nanos: i128) -> Option<String> {
        self.local_datetime(epoch_nanos)
            .map(|value| value.format("%A").to_string())
    }

    pub(crate) fn observed_day_span(&self, first: i128, last: i128) -> u64 {
        let Some(first_date) = self.local_datetime(first).map(|value| value.date_naive()) else {
            return 0;
        };
        let Some(last_date) = self.local_datetime(last).map(|value| value.date_naive()) else {
            return 0;
        };
        let days = last_date
            .signed_duration_since(first_date)
            .num_days()
            .max(0);
        u64::try_from(days).unwrap_or(u64::MAX).saturating_add(1)
    }

    pub(crate) fn same_local_day(&self, start: i128, end: i128) -> bool {
        match (self.local_datetime(start), self.local_datetime(end)) {
            (Some(start), Some(end)) => start.date_naive() == end.date_naive(),
            _ => false,
        }
    }

    pub(crate) fn local_day_boundaries(&self) -> Result<Vec<i128>, TimeContextError> {
        let Some(year) = self.year else {
            return Ok(Vec::new());
        };
        let mut date = NaiveDate::from_ymd_opt(year, 1, 1).ok_or_else(|| TimeContextError {
            code: "E_PERIOD_INVALID",
            message: "the selected year is outside the supported calendar range".to_string(),
            remediation: "Choose a four-digit year supported by the IANA calendar.",
        })?;
        let end = NaiveDate::from_ymd_opt(year + 1, 1, 1).ok_or_else(|| TimeContextError {
            code: "E_PERIOD_INVALID",
            message: "the selected year cannot form local-day boundaries".to_string(),
            remediation: "Choose a four-digit year supported by the IANA calendar.",
        })?;
        let mut boundaries = Vec::with_capacity(367);
        while date <= end {
            boundaries.push(local_date_start_nanos(self.timezone, date)?);
            date = date.succ_opt().ok_or_else(|| TimeContextError {
                code: "E_PERIOD_INVALID",
                message: "the selected period exceeded the supported calendar".to_string(),
                remediation: "Choose a four-digit year supported by the IANA calendar.",
            })?;
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        Ok(boundaries)
    }

    fn local_datetime(&self, epoch_nanos: i128) -> Option<DateTime<Tz>> {
        epoch_datetime(epoch_nanos).map(|instant| instant.with_timezone(&self.timezone))
    }
}

fn local_date_start_nanos(timezone: Tz, date: NaiveDate) -> Result<i128, TimeContextError> {
    for minute in 0..=26 * 60 {
        let local = date
            .and_hms_opt(0, 0, 0)
            .and_then(|value| value.checked_add_signed(Duration::minutes(minute)))
            .ok_or_else(|| TimeContextError {
                code: "E_PERIOD_INVALID",
                message: "the selected local date cannot be represented".to_string(),
                remediation: "Choose a supported reporting year and IANA timezone.",
            })?;
        let instant = match timezone.from_local_datetime(&local) {
            LocalResult::Single(value) => Some(value),
            LocalResult::Ambiguous(left, right) => Some(left.min(right)),
            LocalResult::None => None,
        };
        if let Some(instant) = instant {
            return instant
                .with_timezone(&Utc)
                .timestamp_nanos_opt()
                .map(i128::from)
                .ok_or_else(|| TimeContextError {
                    code: "E_PERIOD_INVALID",
                    message: "the selected local boundary exceeds timestamp precision".to_string(),
                    remediation: "Choose a reporting year closer to the Unix epoch.",
                });
        }
    }
    Err(TimeContextError {
        code: "E_PERIOD_INVALID",
        message: "the selected local date has no representable instant in the timezone".to_string(),
        remediation: "Choose a supported reporting year and IANA timezone.",
    })
}

pub(crate) fn epoch_datetime(epoch_nanos: i128) -> Option<DateTime<Utc>> {
    let seconds = epoch_nanos.div_euclid(NANOS_PER_SECOND);
    let subsecond = epoch_nanos.rem_euclid(NANOS_PER_SECOND);
    DateTime::<Utc>::from_timestamp(i64::try_from(seconds).ok()?, u32::try_from(subsecond).ok()?)
}

#[cfg(test)]
mod tests {
    use super::super::types::Diagnostics;
    use super::TimeContext;

    #[test]
    fn default_timezone_uses_valid_host_zone_and_warns_on_utc_fallback() {
        let (host, host_fallback) =
            TimeContext::resolve_default_from(Some("Europe/Berlin"), Some(2026));
        assert_eq!(host.name(), "Europe/Berlin");
        assert!(!host_fallback);

        for unavailable_host in [None, Some("Not/A_Real_Zone")] {
            let (utc, fallback) = TimeContext::resolve_default_from(unavailable_host, Some(2026));
            assert_eq!(utc.name(), "UTC");
            assert!(fallback);
            let coverage = Diagnostics::default().finalize(&utc, fallback);
            assert_eq!(coverage.timezone, "UTC");
            assert!(coverage.warnings.iter().any(|warning| {
                warning.code == "W_TIMEZONE_DEFAULTED_TO_UTC" && warning.source_alias.is_none()
            }));
        }
    }
}
