//! The limit on writes over MCP: at most `MAX_WRITES` calls to tools that change something in any `WINDOW_MINUTES`
//! minutes per person, whatever the tool and whichever of the person's connections. It bounds what a planted
//! instruction can do before a person notices. Held in memory: a restart forgets it, and one server process runs
//! per instance (kb/ADR-0029, kb/ADR-0016 R3).
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

pub(crate) const MAX_WRITES: usize = 30;
const WINDOW_MINUTES: i64 = 10;

/// Each person's counted calls of the last `WINDOW_MINUTES` minutes, in the order they arrived, which need not be
/// the order of their instants: a request's instant is read before its token and body are.
#[derive(Default)]
pub(crate) struct WriteLimit(Mutex<HashMap<String, Vec<DateTime<Utc>>>>);

impl WriteLimit {
    /// Counts one call by `person` at `now`, or refuses it with the instant from which a call will pass: the
    /// oldest counted call's instant plus the window. Forgetting what is older than the window, the check and the
    /// count are one step under the lock. A refused call is not counted. Only the caller's own calls are looked at,
    /// so the map holds one short list for each person who has written since the process started.
    pub(crate) fn reserve(&self, person: &str, now: DateTime<Utc>) -> Result<(), DateTime<Utc>> {
        let window = Duration::minutes(WINDOW_MINUTES);
        // A panic while the lock was held leaves nothing half-written here: go on with what is there.
        let mut calls = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let log = calls.entry(person.to_string()).or_default();
        log.retain(|at| *at + window > now);
        match log.iter().min() {
            Some(oldest) if log.len() >= MAX_WRITES => Err(*oldest + window),
            _ => {
                log.push(now);
                Ok(())
            }
        }
    }
}

/// What the model is told when a call is over the limit: that nothing was written, and when to try again, as a
/// number of seconds (a model has no clock) and as a time. Part of a second is rounded up.
pub(crate) fn told(again: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = ((again - now).num_milliseconds().max(0) + 999) / 1000;
    format!(
        "Too many changes: one person may make at most {MAX_WRITES} calls to tools that change the planner in {WINDOW_MINUTES} minutes. Nothing was written. Try again in {seconds} seconds (at {} UTC).",
        again.format("%H:%M:%S")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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
                limit.reserve("dan@example.com", at(0, 0)),
                Ok(()),
                "call {n}"
            );
        }
        assert_eq!(limit.reserve("dan@example.com", at(0, 0)), Err(at(10, 0)));
        assert_eq!(
            limit.reserve("dan@example.com", at(9, 59)),
            Err(at(10, 0)),
            "one second early"
        );
        // Another person has a count of their own.
        assert_eq!(limit.reserve("bob@example.com", at(9, 59)), Ok(()));
        assert_eq!(limit.reserve("dan@example.com", at(10, 0)), Ok(()));
    }

    /// "Any ten minutes": half the calls at 0:00 and half at 5:00, alternating. At 10:00 the first half is old,
    /// the second is not, so fifteen more pass and the next waits for 15:00. A counter that restarts every ten
    /// minutes would let thirty pass at 10:00, and a log pruned only from its front would forget nothing here.
    #[test]
    fn the_window_slides() {
        let limit = WriteLimit::default();
        for _ in 0..15 {
            limit.reserve("dan@example.com", at(0, 0)).unwrap();
            limit.reserve("dan@example.com", at(5, 0)).unwrap();
        }
        for n in 0..15 {
            assert_eq!(
                limit.reserve("dan@example.com", at(10, 0)),
                Ok(()),
                "call {n}"
            );
        }
        assert_eq!(limit.reserve("dan@example.com", at(10, 0)), Err(at(15, 0)));
    }

    /// A request's instant is read before its body, so reservations can arrive out of time order. The time a
    /// refusal names is the oldest counted call's plus the window, wherever in the log that call sits.
    #[test]
    fn calls_that_arrive_out_of_time_order_are_counted_by_their_own_instants() {
        let limit = WriteLimit::default();
        for _ in 0..MAX_WRITES - 1 {
            limit.reserve("dan@example.com", at(0, 5)).unwrap();
        }
        // The thirtieth to arrive is the oldest.
        limit.reserve("dan@example.com", at(0, 0)).unwrap();
        assert_eq!(limit.reserve("dan@example.com", at(0, 5)), Err(at(10, 0)));
        // At 10:00 only that one is old: one call passes, and the next waits for the others.
        assert_eq!(limit.reserve("dan@example.com", at(10, 0)), Ok(()));
        assert_eq!(limit.reserve("dan@example.com", at(10, 0)), Err(at(10, 5)));
    }

    /// A refused call is not counted: thirty calls, then a hundred refused ones, and the person is still let
    /// through the moment the first call is ten minutes old.
    #[test]
    fn a_refused_call_is_not_counted() {
        let limit = WriteLimit::default();
        for _ in 0..MAX_WRITES {
            limit.reserve("dan@example.com", at(0, 0)).unwrap();
        }
        for _ in 0..100 {
            assert!(limit.reserve("dan@example.com", at(5, 0)).is_err());
        }
        assert_eq!(limit.reserve("dan@example.com", at(10, 0)), Ok(()));
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
                            .filter(|_| limit.reserve("dan@example.com", at(0, 0)).is_ok())
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
            "Too many changes: one person may make at most 30 calls to tools that change the planner in 10 minutes. Nothing was written. Try again in 95 seconds (at 09:10:00 UTC)."
        );
        // A part of a second is rounded up: a call told to wait 1 second and waiting it must pass.
        assert!(told(at(10, 0), at(10, 0) - Duration::milliseconds(1)).contains("in 1 seconds"));
    }
}
