//! Every domain-to-wire conversion for planning: the domain never names a DTO (the isolation lint enforces it).
use super::urgent::WarRoomEntry;
use crate::domain::decision::RequiredDecision;
use crate::domain::items::{Epic, Initiative, Story};
use crate::domain::promise::{self, Clock, HeldBy, Promise, SprintPromise, StoryPromise, epic_due};
use crate::domain::pull_request::PullRequest;
use crate::domain::queue::{QueueEntry, QueueStory, QueueSummary};
use crate::domain::sprint::{Goal, GoalLink, Sprint, SprintCounts, Summary, warnings};
use crate::domain::urgency::{MarkState, Urgency};
use chrono::NaiveDate;
use reeve_protocol::{
    AffectedStoryDto, ClosedDto, CriterionDto, DecisionQueueEntryDto, DecisionQueueSummaryDto,
    DemoStoryDto, DiscoveryMarkDto, EpicDto, EpicPromiseDto, EpicPromiseRefDto, GoalDto,
    GoalProgressDto, InitiativeDto, Key, PromiseDue, PromiseFactDto, PromiseHolder, PromiseState,
    PromisedFilter, PullRequestDto, RecordRefDto, RequiredDecisionDto, SprintCountsDto, SprintDto,
    SprintPromiseDto, SprintState, SprintViewDto, StoryDto, StoryPromiseDto,
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
            promise: None,
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
            promise: None,
        }
    }
}

impl From<&RequiredDecision> for RequiredDecisionDto {
    fn from(d: &RequiredDecision) -> Self {
        Self {
            id: d.id(),
            on: d.on().clone(),
            on_type: d.on_type(),
            title: d.title().to_string(),
            decider: d.decider().to_string(),
            due: d.due(),
            reversibility: d.reversibility(),
            record: d.record().map(|r| RecordRefDto {
                repo: r.repo().to_string(),
                path: r.path().to_string(),
                record_id: r.record_id().to_string(),
                // The server derives it from configuration; planning knows repository names only.
                browse_url: None,
            }),
            pr_url: d.pr_url().map(str::to_string),
            check: d.check().clone(),
            checked_at: d.checked_at(),
            closed: d.closed().map(|c| ClosedDto {
                at: c.at,
                by: c.by.clone(),
                reason: c.reason.clone(),
                kind: c.kind,
                via: c.via,
            }),
            created_at: d.created_at(),
            version: d.version(),
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

impl From<&QueueStory> for AffectedStoryDto {
    fn from(b: &QueueStory) -> Self {
        Self {
            key: b.key.clone(),
            title: b.title.clone(),
            kind: b.kind,
            delivery: b.delivery,
            owner: b.owner.clone(),
            urgent: b.urgent,
            needed_in: b.needed_in,
        }
    }
}

impl From<&QueueEntry> for DecisionQueueEntryDto {
    fn from(e: &QueueEntry) -> Self {
        Self {
            decision: RequiredDecisionDto::from(&e.decision),
            group: e.group,
            lane: e.lane,
            needed_in: e.needed_in,
            needed_in_reason: e.needed_in_reason.clone(),
            next_step: e.next_step.clone(),
            blocks: e.held.iter().map(AffectedStoryDto::from).collect(),
            under_way: e.under_way.iter().map(AffectedStoryDto::from).collect(),
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
            review: s.review,
            this_sprint: s.this_sprint,
            next_sprint: s.next_sprint,
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
    promises: &[SprintPromise],
    promises_after: Option<NaiveDate>,
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
        promises: promises.iter().map(sprint_promise_dto).collect(),
        promises_after,
    }
}

/// `held_by` on the wire: the holder, and the epic's key when the epic holds.
fn holder_of(h: &HeldBy) -> (PromiseHolder, Option<Key>) {
    match h {
        HeldBy::Own => (PromiseHolder::Own, None),
        HeldBy::Epic(k) => (PromiseHolder::Epic, Some(k.clone())),
    }
}

/// One row of a sprint's promise list on the wire. `holder` and `epic_key` both come from `held_by`, so an epic holder
/// always names its epic.
pub fn sprint_promise_dto(p: &SprintPromise) -> SprintPromiseDto {
    let (holder, epic_key) = holder_of(p.held_by());
    SprintPromiseDto {
        key: p.key().clone(),
        title: p.title().to_string(),
        on: p.on(),
        to: p.to().map(str::to_string),
        holder,
        epic_key,
        state: p.state().into(),
        done_on: p.done_on(),
        days: p.days(),
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

/// One duplicate-check hit: what a reporter needs to recognise the story, and whether it was dropped.
impl From<&Story> for reeve_protocol::SimilarStory {
    fn from(s: &Story) -> Self {
        Self {
            key: s.key().clone(),
            kind: s.kind(),
            title: s.title().to_string(),
            epic: s.epic().cloned(),
            delivery: s.delivery(),
            resolution: s.resolution(),
        }
    }
}

/// The one place an `AttachmentDto` is built, its `url` included (kb/ADR-0023 R4: served by the server only).
pub fn attachment_dto(a: &crate::domain::attachment::Attachment) -> reeve_protocol::AttachmentDto {
    use crate::domain::attachment::AttachmentOwner;
    reeve_protocol::AttachmentDto {
        id: a.id(),
        url: format!("/api/v1/attachments/{}", a.id()),
        filename: a.filename().to_string(),
        content_type: a.image_type().mime().to_string(),
        byte_size: a.byte_size(),
        uploaded_by: a.uploaded_by().to_string(),
        uploaded_at: a.uploaded_at(),
        owner: match a.owner() {
            AttachmentOwner::Item(item, key) => Some(reeve_protocol::AttachmentOwnerDto {
                item: *item,
                key: key.clone(),
            }),
            AttachmentOwner::Unclaimed => None,
        },
    }
}

impl From<&Promise> for PromiseFactDto {
    fn from(p: &Promise) -> Self {
        Self {
            on: p.on(),
            by: p.by().to_owned(),
            at: p.at(),
            to: p.to().map(str::to_owned),
        }
    }
}

impl From<promise::State> for PromiseState {
    fn from(s: promise::State) -> Self {
        match s {
            promise::State::Open => PromiseState::Open,
            promise::State::Overdue => PromiseState::Overdue,
            promise::State::Kept => PromiseState::Kept,
            promise::State::Late => PromiseState::Late,
            promise::State::Dropped => PromiseState::Dropped,
        }
    }
}

impl From<promise::Due> for PromiseDue {
    fn from(d: promise::Due) -> Self {
        match d {
            promise::Due::ThisSprint => PromiseDue::ThisSprint,
            promise::Due::NextSprint => PromiseDue::NextSprint,
            promise::Due::Later => PromiseDue::Later,
        }
    }
}

/// The story list's `promised` parameter as the domain's filter; `Filter::matches` is the only rule that classifies.
impl From<PromisedFilter> for promise::Filter {
    fn from(f: PromisedFilter) -> Self {
        match f {
            PromisedFilter::ThisSprint => promise::Filter::ThisSprint,
            PromisedFilter::NextSprint => promise::Filter::NextSprint,
            PromisedFilter::Overdue => promise::Filter::Overdue,
            PromisedFilter::Any => promise::Filter::Any,
        }
    }
}

/// A story DTO's `promise`: the domain's derived view (`story_promise`) on the wire, or None without one.
pub fn story_promise_field(sp: Option<&StoryPromise>) -> Option<StoryPromiseDto> {
    sp.map(|sp| StoryPromiseDto {
        own: sp.own().map(Into::into),
        epic: sp.epic().map(|(k, p)| EpicPromiseRefDto {
            key: k.clone(),
            fact: p.into(),
        }),
        on: sp.on(),
        holder: holder_of(&sp.held_by()).0,
        state: sp.state().map(Into::into),
        due: sp.due().map(Into::into),
        days: sp.days(),
    })
}

/// An epic DTO's `promise`: its own promise with its due period while not past (`epic_due`), never a state (R4).
pub fn epic_promise_field(p: Option<&Promise>, clock: &Clock) -> Option<EpicPromiseDto> {
    p.map(|p| {
        let due = epic_due(p, clock);
        EpicPromiseDto {
            fact: p.into(),
            due: due.map(Into::into),
            past: due.is_none(),
        }
    })
}

impl From<reeve_protocol::PromiseShortcut> for promise::Shortcut {
    fn from(s: reeve_protocol::PromiseShortcut) -> Self {
        match s {
            reeve_protocol::PromiseShortcut::ThisSprint => promise::Shortcut::ThisSprint,
            reeve_protocol::PromiseShortcut::NextSprint => promise::Shortcut::NextSprint,
        }
    }
}

/// The request body as the domain's request: `to` passes through untouched (None keeps, `""` clears).
impl From<reeve_protocol::SetPromise> for promise::Request {
    fn from(b: reeve_protocol::SetPromise) -> Self {
        promise::Request {
            on: b.on,
            shortcut: b.shortcut.map(Into::into),
            to: b.to,
        }
    }
}
