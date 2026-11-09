//! Every domain-to-wire conversion for planning: the domain never names a DTO (the isolation lint enforces it).
use super::urgent::WarRoomEntry;
use crate::domain::decision::RequiredDecision;
use crate::domain::items::{Epic, Initiative, Story};
use crate::domain::pull_request::PullRequest;
use crate::domain::queue::{BlockedStory, QueueEntry, QueueSummary};
use crate::domain::sprint::{Goal, GoalLink, Sprint, SprintCounts, Summary, warnings};
use crate::domain::urgency::{MarkState, Urgency};
use reeve_protocol::{
    BlockedStoryDto, ClosedDto, CriterionDto, DecisionQueueEntryDto, DecisionQueueSummaryDto,
    DemoStoryDto, DiscoveryMarkDto, EpicDto, GoalDto, GoalProgressDto, InitiativeDto, Key,
    PullRequestDto, RecordRefDto, RequiredDecisionDto, SprintCountsDto, SprintDto, SprintState,
    SprintViewDto, StoryDto,
};

/// `progress` is None; the handler fills it from story counts.
impl From<&Initiative> for InitiativeDto {
    fn from(i: &Initiative) -> Self {
        Self {
            key: i.key().clone(),
            title: i.title().to_owned(),
            brief: i.brief().to_owned(),
            horizon: i.horizon().to_owned(),
            target_month: i.target_month().map(str::to_owned),
            owner: i.owner().map(str::to_owned),
            discovery_complete: i.discovery().map(|d| DiscoveryMarkDto {
                at: d.at,
                by: d.by.clone(),
            }),
            progress: None,
            external_ref: i.external_ref().map(str::to_owned),
            version: i.version(),
            urgent_mark: None,
            urgent_sources: vec![],
        }
    }
}
impl From<&Epic> for EpicDto {
    fn from(e: &Epic) -> Self {
        Self {
            key: e.key().clone(),
            title: e.title().to_owned(),
            initiative: e.initiative().cloned(),
            brief: e.brief().to_owned(),
            discovery_complete: e.discovery().map(|d| DiscoveryMarkDto {
                at: d.at,
                by: d.by.clone(),
            }),
            external_ref: e.external_ref().map(str::to_owned),
            version: e.version(),
            urgent_mark: None,
            urgent_sources: vec![],
        }
    }
}
/// `blocked_by`, `prs` and `goals` are empty; the handler fills them from the repo.
impl From<&Story> for StoryDto {
    fn from(s: &Story) -> Self {
        Self {
            key: s.key().clone(),
            title: s.title().to_owned(),
            kind: s.kind(),
            epic: s.epic().cloned(),
            description: s.description().to_owned(),
            criteria: s
                .criteria()
                .iter()
                .map(|c| CriterionDto {
                    text: c.text.clone(),
                    done: c.done,
                })
                .collect(),
            owner: s.owner().map(str::to_owned),
            labels: s.labels().to_vec(),
            rank: s.rank(),
            blocked_by: vec![],
            delivery: s.delivery(),
            resolution: s.resolution(),
            external_ref: s.external_ref().map(str::to_owned),
            version: s.version(),
            prs: vec![],
            goals: vec![],
            urgent_mark: None,
            urgent_sources: vec![],
        }
    }
}

impl From<&RequiredDecision> for RequiredDecisionDto {
    fn from(d: &RequiredDecision) -> Self {
        Self {
            id: d.id,
            on: d.on.clone(),
            on_type: d.on_type,
            title: d.title.clone(),
            decider: d.decider.clone(),
            due: d.due,
            reversibility: d.reversibility,
            record: d.record.as_ref().map(|r| RecordRefDto {
                repo: r.repo.clone(),
                path: r.path.clone(),
                record_id: r.record_id.clone(),
            }),
            pr_url: d.pr_url.clone(),
            check: d.check.clone(),
            checked_at: d.checked_at,
            closed: d.closed.as_ref().map(|c| ClosedDto {
                at: c.at,
                by: c.by.clone(),
                reason: c.reason.clone(),
                kind: c.kind,
                via: c.via,
            }),
            created_at: d.created_at,
            version: d.version,
        }
    }
}

impl From<&PullRequest> for PullRequestDto {
    fn from(p: &PullRequest) -> Self {
        Self {
            repo: p.repo.clone(),
            number: p.number,
            url: p.url.clone(),
            title: p.title.clone(),
            status: p.status,
            updated_at: p.updated_at,
        }
    }
}

impl From<&BlockedStory> for BlockedStoryDto {
    fn from(b: &BlockedStory) -> Self {
        Self {
            key: b.key.clone(),
            title: b.title.clone(),
        }
    }
}

impl From<&QueueEntry> for DecisionQueueEntryDto {
    fn from(e: &QueueEntry) -> Self {
        Self {
            decision: RequiredDecisionDto::from(&e.decision),
            group: e.group,
            blocks: e.blocks.iter().map(BlockedStoryDto::from).collect(),
        }
    }
}

impl From<QueueSummary> for DecisionQueueSummaryDto {
    fn from(s: QueueSummary) -> Self {
        Self {
            open: s.open,
            blocking: s.blocking,
            blocked_stories: s.blocked_stories,
            overdue: s.overdue,
        }
    }
}

/// The sprint's counts on the wire; every field of both DTOs is assigned (no `..`).
impl From<&SprintCounts> for SprintCountsDto {
    fn from(c: &SprintCounts) -> Self {
        Self {
            completed: c.completed,
            upkeep_completed: c.upkeep_completed,
            goals: c
                .goals
                .iter()
                .map(|g| GoalProgressDto {
                    goal_id: g.goal_id,
                    completed: g.completed,
                    in_flight: g.in_flight,
                    todo: g.todo,
                    in_review: g.in_review,
                    blocked_decisions: g.blocked_decisions,
                })
                .collect(),
        }
    }
}

/// A goal on the wire, with its links (any order in `links`; only this goal's are taken, in their given order).
pub fn goal_dto(g: &Goal, links: &[GoalLink]) -> GoalDto {
    GoalDto {
        id: g.id,
        sprint_id: g.sprint_id,
        outcome: g.outcome.clone(),
        check: g.check.clone(),
        owner: g.owner.clone(),
        notes: g.notes.clone(),
        confidence: g.confidence,
        confidence_note: g.confidence_note.clone(),
        confidence_at: g.confidence_at,
        suggested_confidence: g.suggested_confidence,
        suggested_note: g.suggested_note.clone(),
        suggested_at: g.suggested_at,
        suggested_by: g.suggested_by.clone(),
        verdict: g.verdict,
        verdict_note: g.verdict_note.clone(),
        version: g.version,
        links: links
            .iter()
            .filter(|l| l.goal_id == g.id)
            .map(|l| l.item_key.clone())
            .collect(),
    }
}

/// A sprint on the wire with its goals (taken from `goals` by sprint id) and, for a draft or the active sprint, the
/// warnings of its goal count.
pub fn sprint_dto(s: &Sprint, goals: &[Goal], links: &[GoalLink]) -> SprintDto {
    let goals: Vec<GoalDto> = goals
        .iter()
        .filter(|g| g.sprint_id == s.id)
        .map(|g| goal_dto(g, links))
        .collect();
    SprintDto {
        id: s.id,
        state: s.state,
        starts_on: s.starts_on,
        ends_on: s.ends_on,
        opened_at: s.opened_at,
        closed_at: s.closed_at,
        version: s.version,
        warnings: match s.state {
            SprintState::Closed => vec![],
            SprintState::Draft | SprintState::Active => warnings(goals.len()),
        },
        goals,
    }
}

/// A sprint with its numbers; `summary` is None for a draft (no window), which shows zero counts and no demo list.
pub fn sprint_view_dto(
    s: &Sprint,
    goals: &[Goal],
    links: &[GoalLink],
    summary: Option<&Summary>,
    upkeep_line: u8,
) -> SprintViewDto {
    SprintViewDto {
        sprint: sprint_dto(s, goals, links),
        counts: summary
            .map(|x| SprintCountsDto::from(&x.counts))
            .unwrap_or_default(),
        upkeep_line,
        demo: summary
            .map(|x| {
                x.demo
                    .iter()
                    .map(|d| DemoStoryDto {
                        key: d.key.clone(),
                        title: d.title.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// The two urgency fields every item DTO carries: the mark stored on `key` (active or expired) and the active marks that
/// make it urgent.
pub(crate) fn urgent_fields(
    u: &Urgency,
    key: &Key,
) -> (
    Option<reeve_protocol::UrgentMarkDto>,
    Vec<reeve_protocol::ItemRefDto>,
) {
    let sources = u
        .sources(key)
        .iter()
        .map(|(k, t)| reeve_protocol::ItemRefDto {
            key: k.clone(),
            item: *t,
        })
        .collect();
    (u.mark(key).map(Into::into), sources)
}

impl From<&MarkState> for reeve_protocol::UrgentMarkDto {
    fn from(m: &MarkState) -> Self {
        Self {
            at: m.mark.at,
            by: m.mark.by.clone(),
            reason: m.mark.reason.clone(),
            expires_at: m.expires_at,
            active: m.active,
            done: m.done,
        }
    }
}

impl From<&WarRoomEntry> for reeve_protocol::WarRoomEntryDto {
    fn from(e: &WarRoomEntry) -> Self {
        Self {
            item: reeve_protocol::ItemRefDto {
                key: e.key.clone(),
                item: e.item,
            },
            title: e.title.clone(),
            initiative: e
                .initiative
                .as_ref()
                .map(|i| reeve_protocol::WarRoomInitiativeDto {
                    key: i.key.clone(),
                    title: i.title.clone(),
                    horizon: i.horizon.clone(),
                }),
            mark: (&e.state).into(),
        }
    }
}
