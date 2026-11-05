use reeve_protocol::{DeliveryState, Resolution, TokenKind};

#[derive(Debug, Clone)]
pub struct MoveRequest {
    pub from: DeliveryState,
    /// The story's resolution before the move (set only when `from` is done).
    pub current_resolution: Option<Resolution>,
    pub to: DeliveryState,
    pub ready: bool,
    pub resolution: Option<Resolution>,
    pub override_reason: Option<String>,
    pub token_kind: TokenKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveEffect {
    NoOp,
    Plain,
    Override { reason: String },
    Resolve(Resolution),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MoveRefused {
    #[error("story is not ready")]
    NotReady,
    #[error("automation tokens cannot override the gate")]
    AutomationCannotOverride,
    #[error("an override needs a written reason")]
    OverrideNeedsReason,
    #[error("entering done requires a resolution")]
    ResolutionRequired,
    #[error("completed is only allowed from an active state")]
    CompletedNeedsActiveOrigin,
}

/// The whole delivery policy: two rules, everything else free (kb/ADR-0002 R4).
pub fn validate_move(r: &MoveRequest) -> Result<MoveEffect, MoveRefused> {
    use DeliveryState::*;
    // Done to done changes nothing unless a different resolution is named; then the done rule below applies to it, so a
    // completed story may be re-resolved as dropped, but a dropped one cannot become completed without active work.
    let re_resolve = r.from == Done
        && r.to == Done
        && r.resolution.is_some()
        && r.resolution != r.current_resolution;
    if r.from == r.to && !re_resolve {
        return Ok(MoveEffect::NoOp);
    }
    if r.to == Done {
        return match r.resolution {
            None => Err(MoveRefused::ResolutionRequired),
            Some(Resolution::Completed) if !r.from.is_active() => {
                Err(MoveRefused::CompletedNeedsActiveOrigin)
            }
            Some(res) => Ok(MoveEffect::Resolve(res)),
        };
    }
    let entering_active_from_inactive = r.to.is_active() && !r.from.is_active();
    if entering_active_from_inactive && !r.ready {
        return match (&r.override_reason, r.token_kind) {
            (None, _) => Err(MoveRefused::NotReady),
            (Some(_), TokenKind::Automation) => Err(MoveRefused::AutomationCannotOverride),
            (Some(reason), TokenKind::Interactive) if reason.trim().is_empty() => {
                Err(MoveRefused::OverrideNeedsReason)
            }
            (Some(reason), TokenKind::Interactive) => Ok(MoveEffect::Override {
                reason: reason.trim().to_string(),
            }),
        };
    }
    Ok(MoveEffect::Plain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reeve_protocol::{DeliveryState::*, Resolution, TokenKind};
    fn req(from: reeve_protocol::DeliveryState, to: reeve_protocol::DeliveryState) -> MoveRequest {
        MoveRequest {
            from,
            current_resolution: None,
            to,
            ready: false,
            resolution: None,
            override_reason: None,
            token_kind: TokenKind::Interactive,
        }
    }
    #[test]
    fn ready_story_may_enter_active_from_todo_or_done() {
        for from in [Todo, Done] {
            for to in [InProgress, InReview, Testing] {
                let mut r = req(from, to);
                r.ready = true;
                assert_eq!(validate_move(&r), Ok(MoveEffect::Plain));
            }
        }
    }
    #[test]
    fn unready_story_needs_an_interactive_override_to_enter_active() {
        let r = req(Todo, Testing);
        assert_eq!(validate_move(&r), Err(MoveRefused::NotReady));
        let mut o = req(Todo, Testing);
        o.override_reason = Some("urgent fix".into());
        assert_eq!(
            validate_move(&o),
            Ok(MoveEffect::Override {
                reason: "urgent fix".into()
            })
        );
        let mut auto = o.clone();
        auto.token_kind = TokenKind::Automation;
        assert_eq!(
            validate_move(&auto),
            Err(MoveRefused::AutomationCannotOverride)
        );
        let mut blank = req(Todo, InProgress);
        blank.override_reason = Some("   ".into());
        assert_eq!(validate_move(&blank), Err(MoveRefused::OverrideNeedsReason));
    }
    #[test]
    fn moves_between_active_states_are_free() {
        for (a, b) in [
            (InProgress, InReview),
            (InReview, Testing),
            (Testing, InProgress),
            (InReview, InProgress),
        ] {
            assert_eq!(validate_move(&req(a, b)), Ok(MoveEffect::Plain));
        }
        assert_eq!(
            validate_move(&req(InProgress, Todo)),
            Ok(MoveEffect::Plain),
            "release is free"
        );
    }
    #[test]
    fn done_needs_a_resolution_and_completed_needs_an_active_origin() {
        assert_eq!(
            validate_move(&req(Testing, Done)),
            Err(MoveRefused::ResolutionRequired)
        );
        let mut c = req(Testing, Done);
        c.resolution = Some(Resolution::Completed);
        assert_eq!(
            validate_move(&c),
            Ok(MoveEffect::Resolve(Resolution::Completed))
        );
        let mut from_todo = req(Todo, Done);
        from_todo.resolution = Some(Resolution::Completed);
        assert_eq!(
            validate_move(&from_todo),
            Err(MoveRefused::CompletedNeedsActiveOrigin)
        );
        let mut dropped = req(Todo, Done);
        dropped.resolution = Some(Resolution::Dropped);
        assert_eq!(
            validate_move(&dropped),
            Ok(MoveEffect::Resolve(Resolution::Dropped)),
            "dropping from todo is the one ungated exit"
        );
    }
    #[test]
    fn edge_cases() {
        for from_to in [(Todo, InProgress), (Done, Testing)] {
            for tk in [TokenKind::Interactive, TokenKind::Automation] {
                let mut r = req(from_to.0, from_to.1);
                r.ready = true;
                r.override_reason = Some("why".into());
                r.token_kind = tk;
                assert_eq!(validate_move(&r), Ok(MoveEffect::Plain));
            }
        }
        let mut dd = req(Done, Done);
        dd.current_resolution = Some(Resolution::Dropped);
        dd.resolution = Some(Resolution::Dropped);
        assert_eq!(validate_move(&dd), Ok(MoveEffect::NoOp), "same resolution");
        dd.resolution = None;
        assert_eq!(
            validate_move(&dd),
            Ok(MoveEffect::NoOp),
            "no resolution named"
        );
        dd.resolution = Some(Resolution::Completed);
        assert_eq!(
            validate_move(&dd),
            Err(MoveRefused::CompletedNeedsActiveOrigin),
            "dropped cannot become completed without active work"
        );
        let mut cd = req(Done, Done);
        cd.current_resolution = Some(Resolution::Completed);
        cd.resolution = Some(Resolution::Dropped);
        assert_eq!(
            validate_move(&cd),
            Ok(MoveEffect::Resolve(Resolution::Dropped)),
            "a completed story may be re-resolved as dropped"
        );
        assert_eq!(validate_move(&req(Done, Todo)), Ok(MoveEffect::Plain));
        assert_eq!(
            validate_move(&req(Done, InProgress)),
            Err(MoveRefused::NotReady)
        );
        for from in [InProgress, InReview, Testing] {
            let mut c = req(from, Done);
            c.resolution = Some(Resolution::Completed);
            assert_eq!(
                validate_move(&c),
                Ok(MoveEffect::Resolve(Resolution::Completed))
            );
        }
        let mut a = req(Todo, Testing);
        a.override_reason = Some("  ".into());
        a.token_kind = TokenKind::Automation;
        assert_eq!(
            validate_move(&a),
            Err(MoveRefused::AutomationCannotOverride)
        );
        let mut p = req(Todo, Testing);
        p.override_reason = Some("  urgent  ".into());
        assert_eq!(
            validate_move(&p),
            Ok(MoveEffect::Override {
                reason: "urgent".into()
            })
        );
    }
    #[test]
    fn same_state_is_a_no_op() {
        assert_eq!(validate_move(&req(Todo, Todo)), Ok(MoveEffect::NoOp));
    }
}
