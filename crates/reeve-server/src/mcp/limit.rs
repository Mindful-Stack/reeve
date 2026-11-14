//! The limit on writes over MCP: at most `MAX_WRITES` calls to tools that change something in any `WINDOW_MINUTES`
//! minutes per person, whatever the tool and whichever of the person's connections. It bounds what a planted
//! instruction can do before a person notices. Held in memory: a restart forgets it, and one server process runs
//! per instance (kb/ADR-0029, kb/ADR-0016 R3).
use chrono::{DateTime, TimeDelta, Timelike, Utc};
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

pub(crate) const MAX_WRITES: usize = 30;
const WINDOW_MINUTES: u64 = 10;
const WINDOW: Duration = Duration::from_secs(WINDOW_MINUTES * 60);

/// Each person's counted calls of the last `WINDOW_MINUTES` minutes, in the limit's own time: how far the server's
/// clock has gone forward, in all, between the calls it has seen. A step back of the clock (an NTP step, a resumed
/// VM) adds nothing and is not a pause: from the first call that sees it, the limit's time runs on with the clock,
/// so no one is refused for longer than ten minutes after that call. A step forward only expires calls. Nothing
/// is stored as a calendar date, so no clock, however wrong, makes the limit overflow.
#[derive(Default)]
pub(crate) struct WriteLimit(Mutex<Counted>);

#[derive(Default)]
struct Counted {
    /// The clock's instant at the last call, forward or back.
    wall: Option<DateTime<Utc>>,
    /// The limit's own time.
    time: Duration,
    calls: HashMap<String, Vec<Duration>>,
}

/// A refused call: how long until a call will pass, and the clock's instant it was refused at.
#[derive(Debug, PartialEq)]
pub(crate) struct Over {
    pub(crate) wait: Duration,
    pub(crate) now: DateTime<Utc>,
}

impl WriteLimit {
    /// Counts one call by `person`, or refuses it with how long until a call will pass: until the oldest counted
    /// call is `WINDOW_MINUTES` minutes old in the limit's time. `now` reads the server's clock; it is read under
    /// the lock, so calls are counted in the order they read it and two calls racing for the lock never look like
    /// a step back. Reading the clock, forgetting what is older than the window, the check and the count are one
    /// step under one lock, never held across an await. A refused call is not counted. Only the caller's own calls
    /// are looked at, so the map holds one short list for each person who has written since the process started.
    pub(crate) fn reserve(
        &self,
        person: &str,
        now: impl FnOnce() -> DateTime<Utc>,
    ) -> Result<(), Over> {
        // A panic while the lock was held leaves nothing half-written here: go on with what is there.
        let mut counted = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let now = now();
        if let Some(wall) = counted.wall {
            // A step back is no time at all (`to_std` refuses a negative span).
            let forward = (now - wall).to_std().unwrap_or_default();
            counted.time = counted.time.saturating_add(forward);
        }
        counted.wall = Some(now);
        let time = counted.time;
        let log = counted.calls.entry(person.to_string()).or_default();
        log.retain(|at| time - *at < WINDOW);
        // The limit's time never goes back, so the log is in time order.
        match log.first() {
            Some(oldest) if log.len() >= MAX_WRITES => Err(Over {
                wait: WINDOW - (time - *oldest),
                now,
            }),
            _ => {
                log.push(time);
                Ok(())
            }
        }
    }
}

/// What the model is told when a call is over the limit: that nothing was written, and when to try again, as a
/// number of seconds (a model has no clock) and as a time. The seconds are the wait rounded up, and the time is
/// `now` plus those seconds, rounded up to a whole second, so neither is earlier than when a call will pass. A time
/// the calendar cannot hold is left out.
pub(crate) fn told(wait: Duration, now: DateTime<Utc>) -> String {
    let seconds = wait.as_nanos().div_ceil(1_000_000_000);
    // A wait is at most the window, so the seconds fit.
    let at = now
        .checked_add_signed(TimeDelta::seconds(seconds as i64))
        .and_then(|at| match at.nanosecond() {
            0 => Some(at),
            _ => at
                .checked_add_signed(TimeDelta::seconds(1))?
                .with_nanosecond(0),
        })
        .map(|at| format!(" (at {} UTC)", at.format("%H:%M:%S")))
        .unwrap_or_default();
    format!(
        "Too many changes: one person may make at most {MAX_WRITES} calls to tools that change the planner in {WINDOW_MINUTES} minutes. Nothing was written. Try again in {seconds} seconds{at}. Tell the person; do not call a tool that changes the planner again before then."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const DAN: &str = "dan@example.com";

    /// `WriteLimit::reserve` with only the clock's instant from which a call will pass, by the refusal.
    fn reserve(limit: &WriteLimit, person: &str, now: DateTime<Utc>) -> Result<(), DateTime<Utc>> {
        limit
            .reserve(person, || now)
            .map_err(|over| over.now + TimeDelta::from_std(over.wait).unwrap())
    }

    fn at(minutes: i64, seconds: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 14, 9, 0, 0).unwrap()
            + TimeDelta::minutes(minutes)
            + TimeDelta::seconds(seconds)
    }

    #[test]
    fn the_thirty_first_call_in_ten_minutes_is_refused_until_the_first_is_ten_minutes_old() {
        let limit = WriteLimit::default();
        for n in 0..MAX_WRITES {
            assert_eq!(reserve(&limit, DAN, at(0, 0)), Ok(()), "call {n}");
        }
        assert_eq!(reserve(&limit, DAN, at(0, 0)), Err(at(10, 0)));
        assert_eq!(
            reserve(&limit, DAN, at(9, 59)),
            Err(at(10, 0)),
            "one second early"
        );
        // Another person has a count of their own.
        assert_eq!(reserve(&limit, "bob@example.com", at(9, 59)), Ok(()));
        assert_eq!(reserve(&limit, DAN, at(10, 0)), Ok(()));
    }

    /// "Any ten minutes": half the calls at 0:00 and half at 5:00. At 10:00 the first half is old, the second is
    /// not, so fifteen more pass and the next waits for 15:00. A counter that restarts every ten minutes would let
    /// thirty pass at 10:00.
    #[test]
    fn the_window_slides() {
        let limit = WriteLimit::default();
        for _ in 0..15 {
            reserve(&limit, DAN, at(0, 0)).unwrap();
        }
        for _ in 0..15 {
            reserve(&limit, DAN, at(5, 0)).unwrap();
        }
        for n in 0..15 {
            assert_eq!(reserve(&limit, DAN, at(10, 0)), Ok(()), "call {n}");
        }
        assert_eq!(reserve(&limit, DAN, at(10, 0)), Err(at(15, 0)));
    }

    /// The clock steps back thirty minutes (an NTP step, a resumed VM) when a person is at thirty calls. They are
    /// refused for no longer than ten minutes after the first call that sees the step, and the time they are told
    /// is on the stepped clock and true. A limit that held its time at the latest instant seen would refuse them
    /// for forty minutes and name a time forty minutes ahead.
    #[test]
    fn a_step_back_of_the_clock_refuses_no_one_for_longer_than_ten_minutes_after_it_is_seen() {
        let limit = WriteLimit::default();
        for _ in 0..MAX_WRITES {
            reserve(&limit, DAN, at(30, 0)).unwrap();
        }
        let over = limit.reserve(DAN, || at(0, 0)).unwrap_err();
        assert_eq!(
            over,
            Over {
                wait: WINDOW,
                now: at(0, 0)
            }
        );
        assert!(told(over.wait, over.now).contains("in 600 seconds (at 09:10:00 UTC)"));
        assert_eq!(reserve(&limit, DAN, at(9, 59)), Err(at(10, 0)));
        assert_eq!(reserve(&limit, DAN, at(10, 0)), Ok(()));
        // Thirty more from 10:00 on the stepped clock: the window is the limit's, so the old thirty stay gone and
        // the new ones count.
        for _ in 1..MAX_WRITES {
            reserve(&limit, DAN, at(10, 0)).unwrap();
        }
        assert_eq!(reserve(&limit, DAN, at(10, 0)), Err(at(20, 0)));
    }

    /// A step back shorter than the window is not a pause either: thirty calls at 5:00, the clock steps back to
    /// 0:00, and the person waits ten minutes from the step, told 10:00 on the stepped clock.
    #[test]
    fn a_short_step_back_is_not_a_pause() {
        let limit = WriteLimit::default();
        for _ in 0..MAX_WRITES {
            reserve(&limit, DAN, at(5, 0)).unwrap();
        }
        assert_eq!(reserve(&limit, DAN, at(0, 0)), Err(at(10, 0)));
        assert_eq!(reserve(&limit, DAN, at(9, 59)), Err(at(10, 0)));
        assert_eq!(reserve(&limit, DAN, at(10, 0)), Ok(()));
    }

    /// A clock that goes back and forth, against an oracle written from the rule alone: the limit's time is the sum
    /// of the clock's forward steps between calls, and a call passes when fewer than thirty passed calls are less
    /// than ten minutes of that time old; otherwise it waits until the oldest of them is. Every answer and every
    /// wait told matches, at most thirty pass in any ten minutes of the limit's time, and no one waits longer
    /// than ten minutes.
    #[test]
    fn a_clock_that_goes_back_and_forth_is_answered_as_the_rule_says() {
        let limit = WriteLimit::default();
        let mut wall = at(0, 0);
        let mut previous: Option<DateTime<Utc>> = None;
        let mut time = Duration::ZERO;
        let mut passed: Vec<Duration> = Vec::new();
        for n in 0..3_000_i64 {
            // Forward by 1 to 7 seconds; every third call back by 40 seconds; every 500th back by an hour.
            wall += TimeDelta::seconds(1 + n % 7);
            let now = match n {
                n if n % 500 == 499 => wall - TimeDelta::hours(1),
                n if n % 3 == 0 => wall - TimeDelta::seconds(40),
                _ => wall,
            };
            if let Some(previous) = previous
                && now > previous
            {
                time += (now - previous).to_std().unwrap();
            }
            previous = Some(now);
            let young: Vec<Duration> = passed
                .iter()
                .copied()
                .filter(|at| time - *at < Duration::from_secs(600))
                .collect();
            let expected = if young.len() < 30 {
                passed.push(time);
                Ok(())
            } else {
                Err(Duration::from_secs(600) - (time - young[0]))
            };
            let answer = limit.reserve(DAN, || now);
            assert_eq!(
                answer.as_ref().map_err(|over| over.wait),
                expected.as_ref().map_err(|wait| *wait),
                "call {n}"
            );
            if let (Err(over), Err(wait)) = (answer, expected) {
                assert!(wait > Duration::ZERO && wait <= Duration::from_secs(600));
                let seconds = wait.as_nanos().div_ceil(1_000_000_000);
                assert!(
                    told(over.wait, over.now).contains(&format!("in {seconds} seconds")),
                    "call {n}"
                );
            }
        }
        for (n, first) in passed.iter().enumerate() {
            let within = passed[n..]
                .iter()
                .take_while(|at| **at - *first < Duration::from_secs(600))
                .count();
            assert!(within <= 30, "{within} calls from the {n}th");
        }
        assert!(passed.len() > 3 * MAX_WRITES, "the limit admits again");
        assert!(passed.len() < 3_000, "the limit refuses");
    }

    /// The clock jumps between this year and the year 250 000 and back, again and again: nothing overflows, and
    /// thirty calls at one instant still fill the window.
    #[test]
    fn a_wall_clock_that_jumps_between_years_never_panics_and_still_counts() {
        let limit = WriteLimit::default();
        let far = Utc.with_ymd_and_hms(250_000, 1, 1, 0, 0, 0).unwrap();
        for _ in 0..5 {
            reserve(&limit, DAN, at(0, 0)).unwrap();
            reserve(&limit, DAN, far).unwrap();
            reserve(&limit, DAN, DateTime::<Utc>::MAX_UTC).unwrap();
            reserve(&limit, DAN, DateTime::<Utc>::MIN_UTC).unwrap();
        }
        for _ in 0..MAX_WRITES {
            reserve(&limit, DAN, at(0, 0)).unwrap();
        }
        let over = limit.reserve(DAN, || at(0, 0)).unwrap_err();
        assert_eq!(over.wait, WINDOW);
        for _ in 0..MAX_WRITES {
            reserve(&limit, DAN, far).unwrap();
        }
        assert!(limit.reserve(DAN, || far).is_err());
    }

    /// A refused call is not counted: thirty calls, then a hundred refused ones, and the person is still let
    /// through the moment the first call is ten minutes old.
    #[test]
    fn a_refused_call_is_not_counted() {
        let limit = WriteLimit::default();
        for _ in 0..MAX_WRITES {
            reserve(&limit, DAN, at(0, 0)).unwrap();
        }
        for _ in 0..100 {
            assert!(reserve(&limit, DAN, at(5, 0)).is_err());
        }
        // The clock is read under the limit's lock.
        let read = limit.reserve(DAN, || {
            assert!(limit.0.try_lock().is_err(), "the lock is not held");
            at(10, 0)
        });
        assert_eq!(read, Ok(()));
    }

    /// The reservation is one step: eight threads asking ten times each at one instant get thirty between them.
    #[test]
    fn calls_at_once_cannot_pass_the_limit_together() {
        let limit = WriteLimit::default();
        let passed: usize = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        (0..10)
                            .filter(|_| reserve(&limit, DAN, at(0, 0)).is_ok())
                            .count()
                    })
                })
                .collect();
            threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .sum()
        });
        assert_eq!(passed, MAX_WRITES);
    }

    #[test]
    fn the_model_is_told_when_to_try_again() {
        assert_eq!(
            told(Duration::from_secs(95), at(8, 25)),
            "Too many changes: one person may make at most 30 calls to tools that change the planner in 10 minutes. Nothing was written. Try again in 95 seconds (at 09:10:00 UTC). Tell the person; do not call a tool that changes the planner again before then."
        );
        // A part of a second is rounded up: a call told to wait 1 second and waiting it must pass.
        assert!(told(Duration::from_millis(1), at(10, 0)).contains("in 1 seconds"));
        // The time is now plus the seconds told, rounded up to a whole second: at 9:08:24.5 with 95.2 seconds to
        // wait a call passes at 9:09:59.7, and neither "96 seconds" nor 09:10:01 is earlier than that.
        let half = at(8, 24) + TimeDelta::milliseconds(500);
        assert!(
            told(Duration::from_millis(95_200), half).contains("in 96 seconds (at 09:10:01 UTC)")
        );
    }

    /// Down to a nanosecond: what is left is never told as less than it is.
    #[test]
    fn the_wait_told_is_rounded_up_from_nanoseconds() {
        assert!(
            told(Duration::from_nanos(1), at(10, 0)).contains("in 1 seconds (at 09:10:01 UTC)")
        );
        assert!(
            told(Duration::from_nanos(1_000_000_001), at(10, 0))
                .contains("in 2 seconds (at 09:10:02 UTC)")
        );
    }

    /// Past 2262 (where a time in nanoseconds no longer fits an `i64`) the time is still rounded up, and a time the
    /// calendar cannot hold is left out, with the seconds still told.
    #[test]
    fn a_time_far_from_now_is_told_not_before_admission_or_not_at_all() {
        let late =
            Utc.with_ymd_and_hms(2300, 1, 1, 0, 0, 0).unwrap() + TimeDelta::milliseconds(500);
        assert!(told(Duration::from_secs(10), late).contains("in 10 seconds (at 00:00:11 UTC)"));
        assert!(
            told(Duration::from_secs(10), DateTime::<Utc>::MAX_UTC)
                .contains("in 10 seconds. Tell the person")
        );
    }
}
