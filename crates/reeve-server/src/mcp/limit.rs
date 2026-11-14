//! The limit on writes over MCP: at most `MAX_WRITES` calls to tools that change something in any `WINDOW_MINUTES`
//! minutes per person, whatever the tool and whichever of the person's connections. It bounds what a planted
//! instruction can do before a person notices. Held in memory: a restart forgets it, and one server process runs
//! per instance (kb/ADR-0029, kb/ADR-0016 R3).
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

pub(crate) const MAX_WRITES: usize = 30;
const WINDOW_MINUTES: i64 = 10;

/// Each person's counted calls of the last `WINDOW_MINUTES` minutes, and the latest instant the limit has seen.
/// A request's instant is read before its token and body are, so reservations can arrive out of time order; the
/// limit's own time is the latest instant seen, which never goes back, and every call is counted at it. A late
/// call stamped before an expiry already applied would otherwise sit in the window that expiry closed.
#[derive(Default)]
pub(crate) struct WriteLimit(Mutex<Counted>);

#[derive(Default)]
struct Counted {
    latest: Option<DateTime<Utc>>,
    calls: HashMap<String, Vec<DateTime<Utc>>>,
}

/// A refused call: the instant from which a call will pass, and the limit's time it was refused at.
#[derive(Debug, PartialEq)]
pub(crate) struct Over {
    pub(crate) again: DateTime<Utc>,
    pub(crate) now: DateTime<Utc>,
}

impl WriteLimit {
    /// Counts one call by `person` at `now`, or at the latest instant the limit has seen when that is later, or
    /// refuses it with the instant from which a call will pass: the oldest counted call's instant plus the window.
    /// Forgetting what is older than the window, the check and the count are one step under the lock. A refused
    /// call is not counted. Only the caller's own calls are looked at, so the map holds one short list for each
    /// person who has written since the process started.
    pub(crate) fn reserve(&self, person: &str, now: DateTime<Utc>) -> Result<(), Over> {
        let window = Duration::minutes(WINDOW_MINUTES);
        // A panic while the lock was held leaves nothing half-written here: go on with what is there.
        let mut counted = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let now = counted.latest.map_or(now, |latest| latest.max(now));
        counted.latest = Some(now);
        let log = counted.calls.entry(person.to_string()).or_default();
        log.retain(|at| *at + window > now);
        // Every instant pushed is the limit's time, which never goes back, so the log is in time order.
        match log.first() {
            Some(oldest) if log.len() >= MAX_WRITES => Err(Over {
                again: *oldest + window,
                now,
            }),
            _ => {
                log.push(now);
                Ok(())
            }
        }
    }
}

/// What the model is told when a call is over the limit: that nothing was written, and when to try again, as a
/// number of seconds (a model has no clock) and as a time. Any part of a second is rounded up.
pub(crate) fn told(again: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let micros = (again - now).num_microseconds().unwrap_or(i64::MAX).max(0);
    let seconds = (micros as u64).div_ceil(1_000_000);
    format!(
        "Too many changes: one person may make at most {MAX_WRITES} calls to tools that change the planner in {WINDOW_MINUTES} minutes. Nothing was written. Try again in {seconds} seconds (at {} UTC). Tell the person; do not call a tool that changes the planner again before then.",
        again.format("%H:%M:%S")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// `WriteLimit::reserve` with only the instant a refusal names.
    fn reserve(limit: &WriteLimit, person: &str, now: DateTime<Utc>) -> Result<(), DateTime<Utc>> {
        limit.reserve(person, now).map_err(|over| over.again)
    }

    fn at(minutes: i64, seconds: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 14, 9, 0, 0).unwrap()
            + Duration::minutes(minutes)
            + Duration::seconds(seconds)
    }

    #[test]
    fn the_thirty_first_call_in_ten_minutes_is_refused_until_the_first_is_ten_minutes_old() {
        let limit = WriteLimit::default();
        for n in 0..MAX_WRITES {
            assert_eq!(
                reserve(&limit, "dan@example.com", at(0, 0)),
                Ok(()),
                "call {n}"
            );
        }
        assert_eq!(reserve(&limit, "dan@example.com", at(0, 0)), Err(at(10, 0)));
        assert_eq!(
            reserve(&limit, "dan@example.com", at(9, 59)),
            Err(at(10, 0)),
            "one second early"
        );
        // Another person has a count of their own.
        assert_eq!(reserve(&limit, "bob@example.com", at(9, 59)), Ok(()));
        assert_eq!(reserve(&limit, "dan@example.com", at(10, 0)), Ok(()));
    }

    /// "Any ten minutes": half the calls at 0:00 and half at 5:00. At 10:00 the first half is old, the second is
    /// not, so fifteen more pass and the next waits for 15:00. A counter that restarts every ten minutes would let
    /// thirty pass at 10:00.
    #[test]
    fn the_window_slides() {
        let limit = WriteLimit::default();
        for _ in 0..15 {
            reserve(&limit, "dan@example.com", at(0, 0)).unwrap();
        }
        for _ in 0..15 {
            reserve(&limit, "dan@example.com", at(5, 0)).unwrap();
        }
        for n in 0..15 {
            assert_eq!(
                reserve(&limit, "dan@example.com", at(10, 0)),
                Ok(()),
                "call {n}"
            );
        }
        assert_eq!(
            reserve(&limit, "dan@example.com", at(10, 0)),
            Err(at(15, 0))
        );
    }

    /// A request's instant is read before its body, so reservations can arrive out of time order. The limit's
    /// time never goes back: a call stamped before the latest instant the limit has seen is counted at that
    /// instant, and the time a refusal names follows from it.
    #[test]
    fn a_call_that_arrives_late_is_counted_at_the_latest_instant_seen() {
        let limit = WriteLimit::default();
        for _ in 0..MAX_WRITES - 1 {
            reserve(&limit, "dan@example.com", at(0, 5)).unwrap();
        }
        // The thirtieth to arrive is stamped earlier, and counted at 0:05 with the others.
        reserve(&limit, "dan@example.com", at(0, 0)).unwrap();
        assert_eq!(reserve(&limit, "dan@example.com", at(0, 5)), Err(at(10, 5)));
        assert_eq!(
            reserve(&limit, "dan@example.com", at(10, 0)),
            Err(at(10, 5)),
            "none of the thirty is ten minutes old at 10:00"
        );
        assert_eq!(reserve(&limit, "dan@example.com", at(10, 5)), Ok(()));
    }

    /// Out of order across expiry: thirty calls at 0:00, one at 10:00 forgets them, and then calls stamped 9:59
    /// arrive. Counted at their own instant they would sit in one ten-minute window with the thirty at 0:00 (59
    /// calls between 0:00 and 9:59). Counted at 10:00, the window from 10:00 admits thirty and the next waits for
    /// 20:00.
    #[test]
    fn calls_stamped_before_an_expiry_already_seen_cannot_reopen_the_old_window() {
        let limit = WriteLimit::default();
        for _ in 0..MAX_WRITES {
            reserve(&limit, "dan@example.com", at(0, 0)).unwrap();
        }
        assert_eq!(reserve(&limit, "dan@example.com", at(10, 0)), Ok(()));
        for n in 1..MAX_WRITES {
            assert_eq!(
                reserve(&limit, "dan@example.com", at(9, 59)),
                Ok(()),
                "call {n} of the window from 10:00"
            );
        }
        assert_eq!(
            reserve(&limit, "dan@example.com", at(9, 59)),
            Err(at(20, 0)),
            "the thirty-first since 10:00"
        );
        // The latest instant is the limit's, not each person's: another person's late call is counted at 10:00
        // too, so its window ends at 20:00.
        for _ in 0..MAX_WRITES {
            reserve(&limit, "bob@example.com", at(5, 0)).unwrap();
        }
        assert_eq!(reserve(&limit, "bob@example.com", at(5, 0)), Err(at(20, 0)));
    }

    /// The refusal tells the time from the limit's own instant, not from the late call's.
    #[test]
    fn a_late_call_is_told_to_wait_from_the_latest_instant_seen() {
        let limit = WriteLimit::default();
        for _ in 0..MAX_WRITES {
            reserve(&limit, "dan@example.com", at(1, 0)).unwrap();
        }
        let over = limit.reserve("dan@example.com", at(0, 0)).unwrap_err();
        assert_eq!((over.again, over.now), (at(11, 0), at(1, 0)));
    }

    /// A refused call is not counted: thirty calls, then a hundred refused ones, and the person is still let
    /// through the moment the first call is ten minutes old.
    #[test]
    fn a_refused_call_is_not_counted() {
        let limit = WriteLimit::default();
        for _ in 0..MAX_WRITES {
            reserve(&limit, "dan@example.com", at(0, 0)).unwrap();
        }
        for _ in 0..100 {
            assert!(reserve(&limit, "dan@example.com", at(5, 0)).is_err());
        }
        assert_eq!(reserve(&limit, "dan@example.com", at(10, 0)), Ok(()));
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
                            .filter(|_| reserve(&limit, "dan@example.com", at(0, 0)).is_ok())
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
            told(at(10, 0), at(8, 25)),
            "Too many changes: one person may make at most 30 calls to tools that change the planner in 10 minutes. Nothing was written. Try again in 95 seconds (at 09:10:00 UTC). Tell the person; do not call a tool that changes the planner again before then."
        );
        // A part of a second is rounded up: a call told to wait 1 second and waiting it must pass.
        assert!(told(at(10, 0), at(10, 0) - Duration::milliseconds(1)).contains("in 1 seconds"));
        // Below a millisecond too: 500 microseconds left is not "0 seconds".
        assert!(told(at(10, 0), at(10, 0) - Duration::microseconds(500)).contains("in 1 seconds"));
        assert!(told(at(10, 0), at(10, 0)).contains("in 0 seconds"));
    }
}
