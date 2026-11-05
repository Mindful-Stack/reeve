//! Messages between owners (kb/ADR-0011 R2): a slice's update returns its own effects plus notes, which the core routes.
use crate::tui::effect::{Completion, Effect, Failure, Reply, Stamp};
use reeve_protocol::{Key, StoryDto};

#[expect(
    clippy::large_enum_variant,
    reason = "built once per request and moved through the channel; boxing buys nothing measurable"
)]
#[derive(Debug, Clone, PartialEq)]
pub enum WriteOutcome {
    /// The server answered with the story.
    Applied(StoryDto),
    /// No answer: it may have committed.
    Unknown,
    /// Refused or failed: nothing changed.
    Unchanged,
}

impl WriteOutcome {
    /// The one mapping from a write's completion to what the board learns of it.
    pub fn of(c: &Completion) -> Self {
        match c {
            Completion::Ok(Reply::Story(s)) => WriteOutcome::Applied(s.clone()),
            // A proxy's 5xx can hide a committed write: a server error is as unknown as no answer, and refetching is
            // safe.
            Completion::Failed(Failure::Transport(_) | Failure::Server(_)) => WriteOutcome::Unknown,
            _ => WriteOutcome::Unchanged,
        }
    }
}

#[expect(
    clippy::large_enum_variant,
    reason = "built once per request and moved through the channel; boxing buys nothing measurable"
)]
#[derive(Debug, Clone, PartialEq)]
pub enum Note {
    /// A write on this story is in flight: board holds its lock.
    WriteStarted(Key),
    /// Every write ends with exactly one of these, whatever its outcome.
    WriteFinished {
        key: Key,
        stamp: Stamp,
        outcome: WriteOutcome,
    },
    Refetch,
    /// An activity entry arrived: every read sent so far may predate the change it reports (kb/ADR-0011 R5).
    Invalidate,
    Status(String),
    SignedOut,
    /// The selected story left the board (filter, done window, deleted).
    SelectionLost(Key),
}

pub struct Out<M> {
    pub effects: Vec<Effect<M>>,
    pub notes: Vec<Note>,
}

impl<M: std::fmt::Debug> std::fmt::Debug for Out<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Out")
            .field("effects", &self.effects)
            .field("notes", &self.notes)
            .finish()
    }
}

impl<M> Default for Out<M> {
    fn default() -> Self {
        Self {
            effects: Vec::new(),
            notes: Vec::new(),
        }
    }
}

impl<M: 'static> Out<M> {
    pub fn none() -> Self {
        Self::default()
    }
    pub fn push(&mut self, e: Effect<M>) {
        self.effects.push(e);
    }
    pub fn tell(&mut self, n: Note) {
        self.notes.push(n);
    }
    pub fn status(&mut self, text: impl Into<String>) {
        self.notes.push(Note::Status(text.into()));
    }
    pub fn extend(&mut self, other: Out<M>) {
        self.effects.extend(other.effects);
        self.notes.extend(other.notes);
    }
    pub fn map<N: 'static>(self, f: impl Fn(M) -> N + Send + Sync + Clone + 'static) -> Out<N> {
        Out {
            effects: self.effects.into_iter().map(|e| e.map(f.clone())).collect(),
            notes: self.notes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_write_outcome_follows_from_its_completion() {
        use crate::tui::effect::{Completion, Failure, Reply};
        use crate::tui::fixtures::{refused, story};
        use reeve_protocol::{DeliveryState::Todo, Reason};
        let s = story("ABC-1", Todo, 2).story;
        assert_eq!(
            WriteOutcome::of(&Completion::Ok(Reply::Story(s.clone()))),
            WriteOutcome::Applied(s)
        );
        // A proxy's 5xx can hide a committed write: as unknown as no answer.
        for c in [
            Completion::Failed(Failure::Transport("t".into())),
            Completion::Failed(Failure::Server("502".into())),
        ] {
            assert_eq!(WriteOutcome::of(&c), WriteOutcome::Unknown, "{c:?}");
        }
        for c in [
            refused(vec![Reason::NoCriteria]),
            Completion::Failed(Failure::Conflict),
            Completion::Failed(Failure::Forbidden("no".into())),
            Completion::Failed(Failure::Validation("bad".into())),
            Completion::Failed(Failure::NotFound("gone".into())),
            Completion::Failed(Failure::Unauthenticated),
        ] {
            assert_eq!(WriteOutcome::of(&c), WriteOutcome::Unchanged, "{c:?}");
        }
    }

    #[test]
    fn map_keeps_notes_and_effects() {
        let mut o: Out<u8> = Out::none();
        o.push(Effect::after(Duration::ZERO, 1));
        o.status("hi");
        o.tell(Note::Refetch);
        let m: Out<u16> = o.map(|x| x as u16 + 1);
        assert_eq!(m.effects.len(), 1);
        assert!(matches!(
            m.effects.as_slice(),
            [Effect::After { msg: 2, .. }]
        ));
        assert!(matches!(m.notes.as_slice(), [Note::Status(s), Note::Refetch] if s == "hi"));
    }
}
