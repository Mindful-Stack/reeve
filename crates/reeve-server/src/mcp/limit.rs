//! The limit on writes over MCP: at most `MAX_WRITES` calls to tools that change something in any `WINDOW_MINUTES`
//! minutes per person, whatever the tool and whichever of the person's connections. It bounds what a planted
//! instruction can do before a person notices. Held in memory: a restart forgets it, and one server process runs
//! per instance (kb/ADR-0029, kb/ADR-0016 R3).
use chrono::{DateTime, Duration, DurationRound, Utc};
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

pub(crate) const MAX_WRITES: usize = 30;
const WINDOW_MINUTES: i64 = 10;

/// Each person's counted calls of the last `WINDOW_MINUTES` minutes, in the limit's own time. A call is counted
/// at the instant the server's clock says when it is reserved, after its body was read, so that one person's calls
/// are counted in the order they reach the limit. The limit's own time is the clock's plus how far the clock has
/// stepped back since the process started: it never goes back, and a step back of the clock (an NTP step, a
/// resumed VM) is not a pause. The clock is never trusted to run forward evenly, only to be read: a step forward
/// expires calls early, which no person is refused for.
#[derive(Default)]
pub(crate) struct WriteLimit(Mutex<Counted>);

#[derive(Default)]
struct Counted {
    /// The latest instant of the limit's own time.
    latest: Option<DateTime<Utc>>,
    /// How far the clock has stepped back in all: the clock's instant plus this is the limit's time.
    behind: Duration,
    calls: HashMap<String, Vec<DateTime<Utc>>>,
}

/// A refused call: how long until a call will pass, and the clock's instant it was refused at.
#[derive(Debug, PartialEq)]
pub(crate) struct Over {
    pub(crate) wait: Duration,
    pub(crate) now: DateTime<Utc>,
}

impl WriteLimit {
    /// Counts one call by `person` at `now`, the server's clock read at this call, or refuses it with how long
    /// until a call will pass: until the oldest counted call is `WINDOW_MINUTES` minutes old in the limit's time.
    /// When `now` is earlier than the last instant seen, the clock stepped back, and the step is added to how far
    /// behind it is, so a refused person waits exactly what is left of their window, whatever the clock did.
    /// Forgetting what is older than the window, the check and the count are one step under the lock. A refused
    /// call is not counted. Only the caller's own calls are looked at, so the map holds one short list for each
    /// person who has written since the process started.
    pub(crate) fn reserve(&self, person: &str, now: DateTime<Utc>) -> Result<(), Over> {
        let window = Duration::minutes(WINDOW_MINUTES);
        // A panic while the lock was held leaves nothing half-written here: go on with what is there.
        let mut counted = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut time = now + counted.behind;
        if let Some(latest) = counted.latest
            && time < latest
        {
            counted.behind += latest - time;
            time = latest;
        }
        counted.latest = Some(time);
        let log = counted.calls.entry(person.to_string()).or_default();
        log.retain(|at| *at + window > time);
        // Every instant pushed is the limit's time, which never goes back, so the log is in time order.
        match log.first() {
            Some(oldest) if log.len() >= MAX_WRITES => Err(Over {
                wait: *oldest + window - time,
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
/// number of seconds (a model has no clock) and as a time. Any part of a second is rounded up, and the time is
/// `now` plus those seconds, rounded up to a whole second, so neither is earlier than when a call will pass.
pub(crate) fn told(wait: Duration, now: DateTime<Utc>) -> String {
    let micros = wait.num_microseconds().unwrap_or(i64::MAX).max(0);
    let seconds = (micros as u64).div_ceil(1_000_000);
    // A wait is at most the window, so the seconds fit.
    let at = now + Duration::seconds(seconds as i64);
    let at = at.duration_round_up(Duration::seconds(1)).unwrap_or(at);
    format!(
        "Too many changes: one person may make at most {MAX_WRITES} calls to tools that change the planner in {WINDOW_MINUTES} minutes. Nothing was written. Try again in {seconds} seconds (at {} UTC). Tell the person; do not call a tool that changes the planner again before then.",
        at.format("%H:%M:%S")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// `WriteLimit::reserve` with only the clock's instant from which a call will pass, by the refusal.
    fn reserve(limit: &WriteLimit, person: &str, now: DateTime<Utc>) -> Result<(), DateTime<Utc>> {
        limit
            .reserve(person, now)
            .map_err(|over| over.now + over.wait)
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

    /// Calls are stamped when they are reserved, so sixty cannot pass within a second: thirty at 9:59 and thirty
    /// more at 10:00 are one window, and the second thirty wait for the first to be ten minutes old. (Stamped when
    /// their callers were authenticated, at 0:00, the first thirty would be old at 10:00.)
    #[test]
    fn sixty_calls_cannot_pass_within_a_second() {
        let limit = WriteLimit::default();
        for _ in 0..MAX_WRITES {
            reserve(&limit, "dan@example.com", at(9, 59)).unwrap();
        }
        for n in 0..MAX_WRITES {
            assert_eq!(
                reserve(&limit, "dan@example.com", at(10, 0)),
                Err(at(19, 59)),
                "call {n} at 10:00"
            );
        }
        assert_eq!(reserve(&limit, "dan@example.com", at(19, 59)), Ok(()));
    }

    /// The clock steps back thirty minutes (an NTP step, a resumed VM) when a person is at thirty calls. They are
    /// refused for what is left of their window and no longer, and the time they are told is on the stepped clock:
    /// not in the past and not more than ten minutes ahead. A limit that held its time at the latest instant seen
    /// would refuse them for forty minutes and name a time forty minutes ahead.
    #[test]
    fn a_step_back_of_the_clock_refuses_no_one_for_longer_than_the_window() {
        let limit = WriteLimit::default();
        for _ in 0..MAX_WRITES {
            reserve(&limit, "dan@example.com", at(30, 0)).unwrap();
        }
        let over = limit.reserve("dan@example.com", at(0, 0)).unwrap_err();
        assert_eq!(
            over,
            Over {
                wait: Duration::minutes(10),
                now: at(0, 0)
            }
        );
        assert!(told(over.wait, over.now).contains("in 600 seconds (at 09:10:00 UTC)"));
        assert_eq!(
            reserve(&limit, "dan@example.com", at(9, 59)),
            Err(at(10, 0))
        );
        assert_eq!(reserve(&limit, "dan@example.com", at(10, 0)), Ok(()));
        // Thirty more from 10:00 on the stepped clock: the window is the limit's, so the old thirty stay gone and
        // the new ones count.
        for _ in 1..MAX_WRITES {
            reserve(&limit, "dan@example.com", at(10, 0)).unwrap();
        }
        assert_eq!(
            reserve(&limit, "dan@example.com", at(10, 0)),
            Err(at(20, 0))
        );
    }

    /// A step back shorter than the window is not a pause either: thirty calls at 5:00, the clock steps back to
    /// 0:00, and the person waits ten minutes from the step, told 10:00 on the stepped clock.
    #[test]
    fn a_short_step_back_is_not_a_pause() {
        let limit = WriteLimit::default();
        for _ in 0..MAX_WRITES {
            reserve(&limit, "dan@example.com", at(5, 0)).unwrap();
        }
        assert_eq!(reserve(&limit, "dan@example.com", at(0, 0)), Err(at(10, 0)));
        assert_eq!(
            reserve(&limit, "dan@example.com", at(9, 59)),
            Err(at(10, 0))
        );
        assert_eq!(reserve(&limit, "dan@example.com", at(10, 0)), Ok(()));
    }

    /// In any ten minutes of the limit's own time, at most thirty calls pass, whatever the clock does: calls at
    /// instants that go back and forth by up to a minute, as two threads that read the clock and then race for the
    /// lock would (much exaggerated), never let more than thirty pass in one window of the limit's time.
    #[test]
    fn a_clock_that_goes_back_and_forth_never_lets_more_than_thirty_pass_in_a_window() {
        let limit = WriteLimit::default();
        let mut passed = Vec::new();
        let mut time = at(0, 0);
        for n in 0..2_000_i64 {
            // Forward by 1 to 7 seconds, and every third call back by 40 seconds.
            time += Duration::seconds(1 + n % 7);
            let now = if n % 3 == 0 {
                time - Duration::seconds(40)
            } else {
                time
            };
            if limit.reserve("dan@example.com", now).is_ok() {
                let counted = limit.0.lock().unwrap();
                passed.push(counted.latest.unwrap());
            }
        }
        for (n, first) in passed.iter().enumerate() {
            let within = passed[n..]
                .iter()
                .take_while(|at| **at < *first + Duration::minutes(WINDOW_MINUTES))
                .count();
            assert!(within <= MAX_WRITES, "{within} calls from the {n}th");
        }
        assert!(passed.len() > MAX_WRITES, "the limit admits again");
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
            told(Duration::seconds(95), at(8, 25)),
            "Too many changes: one person may make at most 30 calls to tools that change the planner in 10 minutes. Nothing was written. Try again in 95 seconds (at 09:10:00 UTC). Tell the person; do not call a tool that changes the planner again before then."
        );
        // A part of a second is rounded up: a call told to wait 1 second and waiting it must pass.
        assert!(told(Duration::milliseconds(1), at(10, 0)).contains("in 1 seconds"));
        // Below a millisecond too: 500 microseconds left is not "0 seconds".
        assert!(told(Duration::microseconds(500), at(10, 0)).contains("in 1 seconds"));
        assert!(told(Duration::zero(), at(10, 0)).contains("in 0 seconds (at 09:10:00 UTC)"));
        // The time is now plus the seconds told, rounded up to a whole second: at 9:08:24.5 with 95.2 seconds to
        // wait a call passes at 9:09:59.7, and neither "96 seconds" nor 09:10:01 is earlier than that.
        let half = at(8, 24) + Duration::milliseconds(500);
        assert!(
            told(Duration::milliseconds(95_200), half).contains("in 96 seconds (at 09:10:01 UTC)")
        );
    }
}
