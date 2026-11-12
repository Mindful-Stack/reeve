//! The export readers. Each builds its DTOs from reads on the caller's transaction only, so a
//! `REPEATABLE READ` transaction shared by every reader makes the export one snapshot. The stories go through
//! `story_dtos_in`, the one place a `StoryDto` is built, because it reads on the caller's transaction too. The item
//! readers take the urgency and the promise clock the caller computed once on that transaction (`urgency_in` and
//! `promise_clock_in` at the export time).
use super::epics::epic_dto_of;
use super::initiatives::initiative_dto_of;
use super::*;
use crate::domain::promise::Clock;
use crate::domain::urgency::Urgency;
use reeve_protocol::{
    CommentDto, EpicDto, InitiativeDto, RequiredDecisionDto, Resolution, SprintDto, StoryDto,
};
use sqlx::{Postgres, Transaction};
use std::collections::HashMap;

impl Handlers {
    /// Every initiative, its progress counted from the stories read on this transaction whose epic belongs to it.
    pub async fn export_initiatives(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        urg: &Urgency,
    ) -> Result<Vec<InitiativeDto>, HandlerError> {
        let initiatives = self.ports.reads.initiatives_in(tx).await?;
        let epics = self.ports.reads.epics_in(tx).await?;
        let stories = self.ports.reads.stories_in(tx).await?;
        let initiative_of: HashMap<&Key, &Key> = epics
            .iter()
            .filter_map(|e| e.initiative().map(|i| (e.key(), i)))
            .collect();
        let mut counts: HashMap<&Key, StoryCounts> = HashMap::new();
        for s in &stories {
            let Some(i) = s.epic().and_then(|e| initiative_of.get(e)) else {
                continue;
            };
            let c = counts.entry(i).or_default();
            match s.resolution() {
                Some(Resolution::Completed) => c.completed += 1,
                Some(Resolution::Dropped) => c.dropped += 1,
                None => {}
            }
            c.total += 1;
        }
        Ok(initiatives
            .iter()
            .map(|i| initiative_dto_of(i, &counts.get(i.key()).copied().unwrap_or_default(), urg))
            .collect())
    }

    pub async fn export_epics(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        urg: &Urgency,
        clock: &Clock,
    ) -> Result<Vec<EpicDto>, HandlerError> {
        let epics = self.ports.reads.epics_in(tx).await?;
        let keys: Vec<Key> = epics.iter().map(|e| e.key().clone()).collect();
        let promises = self.ports.reads.promise_facts_in(tx, &[], &keys).await?;
        Ok(epics
            .iter()
            .map(|e| epic_dto_of(e, urg, promises.epics.get(e.key()), clock))
            .collect())
    }

    /// Every story with its blockers and pull requests, through the one assembler, all read on this transaction.
    pub async fn export_stories(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        urg: &Urgency,
        clock: &Clock,
    ) -> Result<Vec<StoryDto>, HandlerError> {
        let stories = self.ports.reads.stories_in(tx).await?;
        self.story_dtos_in(tx, &stories, urg, clock).await
    }

    /// Every sprint, oldest first, with its goals and their links nested, all read on this transaction through the
    /// one `sprint_dto` mapping the sprint routes use.
    pub async fn export_sprints(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<SprintDto>, HandlerError> {
        let mut all = self.ports.sprints.sprints_in(tx).await?;
        // `sprints_in` answers newest first (`ORDER BY id DESC`); the export is oldest first.
        all.sort_by_key(|s| s.id);
        let ids: Vec<i64> = all.iter().map(|s| s.id).collect();
        let goals = self.ports.sprints.goals_of_in(tx, &ids).await?;
        let goal_ids: Vec<i64> = goals.iter().map(|g| g.id).collect();
        let links = self.ports.sprints.links_of_in(tx, &goal_ids).await?;
        Ok(all
            .iter()
            .map(|s| super::dto::sprint_dto(s, &goals, &links))
            .collect())
    }

    pub async fn export_decisions(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<RequiredDecisionDto>, HandlerError> {
        Ok(self
            .ports
            .reads
            .decisions_in(tx)
            .await?
            .iter()
            .map(Into::into)
            .collect())
    }

    pub async fn export_comments(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<CommentDto>, HandlerError> {
        Ok(self.ports.reads.comments_in(tx).await?)
    }

    pub async fn export_attachments(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<reeve_protocol::AttachmentDto>, HandlerError> {
        Ok(self
            .ports
            .reads
            .attachments_in(tx)
            .await?
            .iter()
            .map(super::dto::attachment_dto)
            .collect())
    }
}
