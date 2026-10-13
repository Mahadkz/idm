//! Per-queue schedule arithmetic. Pure functions over an injectable clock so
//! calendar behavior can be tested without sleeping or depending on the
//! machine's time zone.

use std::time::{SystemTime, UNIX_EPOCH};

/// An occurrence without a stop time is only considered startable for this
/// long after its start; a later daemon start skips it rather than running at
/// an arbitrary hour (missed-run policy).
pub const GRACE_SECONDS: i64 = 600;

pub trait Clock: Send + Sync {
    fn now(&self) -> i64;
    /// Local offset from UTC in seconds at the given Unix instant.
    fn utc_offset(&self, at: i64) -> i64;
}

#[cfg(unix)]
unsafe extern "C" {
    fn tzset();
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }

    #[cfg(unix)]
    fn utc_offset(&self, at: i64) -> i64 {
        // SAFETY: localtime_r writes only into the zeroed tm we own.
        unsafe {
            tzset();
            let time = at as libc::time_t;
            let mut parts: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&time, &mut parts).is_null() {
                0
            } else {
                parts.tm_gmtoff as i64
            }
        }
    }

    #[cfg(windows)]
    fn utc_offset(&self, at: i64) -> i64 {
        // The CRT has no tm_gmtoff: the offset is local wall time read as UTC minus the instant.
        // SAFETY: localtime_s writes only into the zeroed tm we own.
        unsafe {
            libc::tzset();
            let time = at as libc::time_t;
            let mut parts: libc::tm = std::mem::zeroed();
            if libc::localtime_s(&mut parts, &time) != 0 {
                return 0;
            }
            let days = days_from_civil(
                parts.tm_year as i64 + 1900,
                parts.tm_mon as i64 + 1,
                parts.tm_mday as i64,
            );
            let wall = days * 86_400
                + parts.tm_hour as i64 * 3600
                + parts.tm_min as i64 * 60
                + parts.tm_sec as i64;
            wall - at
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    None,
    Once,
    Daily,
}

impl Kind {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "none" => Some(Self::None),
            "once" => Some(Self::Once),
            "daily" => Some(Self::Daily),
            _ => None,
        }
    }
}

/// Once: `start`/`stop` are Unix seconds. Daily: local minutes since midnight
/// and `days` is a Monday=bit0..Sunday=bit6 mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schedule {
    pub kind: Kind,
    pub start: i64,
    pub stop: Option<i64>,
    pub days: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Occurrence {
    pub start: i64,
    pub end: i64,
    pub has_stop: bool,
}

pub fn validate(schedule: &Schedule, now: i64) -> Result<(), &'static str> {
    match schedule.kind {
        Kind::None => Ok(()),
        Kind::Once => {
            if schedule.start <= 0 {
                Err("one-time start must be a Unix time")
            } else if schedule.stop.is_some_and(|stop| stop <= schedule.start || stop <= now) {
                Err("stop time must be after the start time and in the future")
            } else {
                Ok(())
            }
        }
        Kind::Daily => {
            if !(0..1440).contains(&schedule.start)
                || schedule
                    .stop
                    .is_some_and(|stop| !(0..1440).contains(&stop) || stop == schedule.start)
            {
                Err("daily times must be distinct minutes within a day")
            } else if schedule.days == 0 || schedule.days > 0x7f {
                Err("select at least one weekday")
            } else {
                Ok(())
            }
        }
    }
}

/// Most recent occurrence whose start is at or before `now`, whether or not
/// its window has already closed. An overnight window (stop <= start) ends on
/// the following local day. DST: local times are converted using the offset in
/// force near the target instant, so a time inside a spring-forward gap lands
/// one hour later and a fall-back fold fires once (identity is the start
/// instant, never repeated).
pub fn current_occurrence(
    schedule: &Schedule,
    now: i64,
    clock: &dyn Clock,
) -> Option<Occurrence> {
    match schedule.kind {
        Kind::None => None,
        Kind::Once => (schedule.start <= now).then(|| Occurrence {
            start: schedule.start,
            end: schedule.stop.unwrap_or(schedule.start + GRACE_SECONDS),
            has_stop: schedule.stop.is_some(),
        }),
        Kind::Daily => {
            let offset_now = clock.utc_offset(now);
            let today = (now + offset_now).div_euclid(86400);
            let to_unix = |local: i64| local - clock.utc_offset(local - offset_now);
            for day in (today - 7..=today).rev() {
                // 1970-01-01 was a Thursday (Monday=0 -> 3).
                let weekday = (day + 3).rem_euclid(7);
                if schedule.days & (1 << weekday) == 0 {
                    continue;
                }
                let start = to_unix(day * 86400 + schedule.start * 60);
                if start > now {
                    continue;
                }
                let end_day = match schedule.stop {
                    Some(stop) if stop <= schedule.start => day + 1,
                    _ => day,
                };
                return Some(match schedule.stop {
                    Some(stop) => Occurrence {
                        start,
                        end: to_unix(end_day * 86400 + stop * 60),
                        has_stop: true,
                    },
                    None => Occurrence {
                        start,
                        end: start + GRACE_SECONDS,
                        has_stop: false,
                    },
                });
            }
            None
        }
    }
}

/// Whether the occurrence should be started now given the last start that was
/// already executed (execution identity).
pub fn should_fire(occurrence: &Occurrence, last_fired: i64, now: i64) -> bool {
    occurrence.start > last_fired && now < occurrence.end
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    /// Fixed-offset clock with settable time.
    pub struct FakeClock {
        pub time: AtomicI64,
        pub offset: i64,
    }

    impl FakeClock {
        pub fn new(time: i64, offset: i64) -> Self {
            Self {
                time: AtomicI64::new(time),
                offset,
            }
        }
        pub fn set(&self, time: i64) {
            self.time.store(time, Ordering::Relaxed);
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> i64 {
            self.time.load(Ordering::Relaxed)
        }
        fn utc_offset(&self, _at: i64) -> i64 {
            self.offset
        }
    }

    // 2024-01-01 was a Monday; local midnight at UTC+2 is 2023-12-31T22:00Z.
    const MONDAY_UTC: i64 = 1_704_067_200;
    const OFFSET: i64 = 2 * 3600;

    fn daily(start: i64, stop: Option<i64>, days: u8) -> Schedule {
        Schedule {
            kind: Kind::Daily,
            start,
            stop,
            days,
        }
    }

    #[test]
    fn validation_rejects_empty_weekdays_and_bad_times() {
        assert!(validate(&daily(60, None, 0), 0).is_err());
        assert!(validate(&daily(1440, None, 1), 0).is_err());
        assert!(validate(&daily(60, Some(60), 1), 0).is_err());
        assert!(validate(&daily(60, Some(120), 0x7f), 0).is_ok());
        let once = |start, stop| Schedule { kind: Kind::Once, start, stop, days: 0 };
        assert!(validate(&once(100, Some(100)), 0).is_err());
        assert!(validate(&once(100, Some(50)), 0).is_err());
        assert!(validate(&once(100, Some(200)), 150).is_ok());
        assert!(validate(&once(100, Some(200)), 250).is_err());
    }

    #[test]
    fn daily_uses_local_time_and_weekdays() {
        // Monday 2024-01-01 local 09:00 is 07:00Z.
        let clock = FakeClock::new(0, OFFSET);
        let schedule = daily(9 * 60, Some(17 * 60), 0b000_0001); // Monday only
        let local_monday = MONDAY_UTC - OFFSET;
        let nine = local_monday + 9 * 3600;
        assert_eq!(current_occurrence(&schedule, nine - 1, &clock).map(|o| o.start), Some(nine - 7 * 86400));
        let occurrence = current_occurrence(&schedule, nine, &clock).unwrap();
        assert_eq!((occurrence.start, occurrence.end), (nine, local_monday + 17 * 3600));
        // Tuesday after the window still reports Monday's closed occurrence.
        let tuesday = current_occurrence(&schedule, nine + 86400, &clock).unwrap();
        assert_eq!(tuesday.start, nine);
        assert!(!should_fire(&tuesday, nine, nine + 86400));
    }

    #[test]
    fn overnight_window_and_missed_window_policy() {
        let clock = FakeClock::new(0, OFFSET);
        let schedule = daily(23 * 60, Some(60), 0x7f);
        let local_monday = MONDAY_UTC - OFFSET;
        let start = local_monday + 23 * 3600;
        let at_half_past_midnight = start + 5400;
        let occurrence = current_occurrence(&schedule, at_half_past_midnight, &clock).unwrap();
        assert_eq!(occurrence.start, start);
        assert_eq!(occurrence.end, start + 7200);
        assert!(should_fire(&occurrence, 0, at_half_past_midnight));
        // Already executed: not repeated; closed window: never started late.
        assert!(!should_fire(&occurrence, start, at_half_past_midnight));
        assert!(!should_fire(&occurrence, 0, start + 7200));
    }

    #[test]
    fn once_without_stop_has_grace_and_with_stop_never_runs_past_it() {
        let clock = FakeClock::new(0, 0);
        let open = Schedule { kind: Kind::Once, start: 1000, stop: None, days: 0 };
        let occurrence = current_occurrence(&open, 1100, &clock).unwrap();
        assert!(should_fire(&occurrence, 0, 1100));
        assert!(!should_fire(&occurrence, 0, 1000 + GRACE_SECONDS));
        let bounded = Schedule { kind: Kind::Once, start: 1000, stop: Some(2000), days: 0 };
        let occurrence = current_occurrence(&bounded, 1999, &clock).unwrap();
        assert!(should_fire(&occurrence, 0, 1999));
        assert!(!should_fire(&occurrence, 0, 2000));
        assert!(current_occurrence(&bounded, 999, &clock).is_none());
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
#[cfg(any(windows, test))]
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod civil_tests {
    #[test]
    fn days_from_civil_matches_known_dates() {
        assert_eq!(super::days_from_civil(1970, 1, 1), 0);
        assert_eq!(super::days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(super::days_from_civil(2026, 10, 13), 20_739);
    }
}
