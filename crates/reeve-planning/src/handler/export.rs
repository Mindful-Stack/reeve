//! The export readers. Each builds its DTOs from reads on the caller's transaction only, so a
//! `REPEATABLE READ` transaction shared by every reader makes the export one snapshot. The stories go through
//! `story_dtos_in`, the one place a `StoryDto` is built, because it reads on the caller's transaction too.
use super::*;
use crate::domain::progress::progress;
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
    ) -> Result<Vec<InitiativeDto>, HandlerError> {
        let initiatives = self.repo.initiatives_in(tx).await?;
        let epics = self.repo.epics_in(tx).await?;
        let stories = self.repo.stories_in(tx).await?;
        let initiative_of: HashMap<&Key, &Key> = epics
            .iter()
            .filter_map(|e| e.initiative().map(|i| (e.key(), i)))
            .collect();
        // (completed, dropped, total) per initiative
        let mut counts: HashMap<&Key, (u32, u32, u32)> = HashMap::new();
        for s in &stories {
            let Some(i) = s.epic().and_then(|e| initiative_of.get(e)) else {
                continue;
            };
            let c = counts.entry(i).or_default();
            match s.resolution() {
                Some(Resolution::Completed) => c.0 += 1,
                Some(Resolution::Dropped) => c.1 += 1,
                None => {}
            }
            c.2 += 1;
        }
        Ok(initiatives
            .iter()
            .map(|i| {
                let (completed, dropped, total) = counts.get(i.key()).copied().unwrap_or_default();
                let mut dto = InitiativeDto::from(i);
                dto.progress = progress(completed, dropped, total);
                dto
            })
            .collect())
    }

    pub async fn export_epics(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<EpicDto>, HandlerError> {
        Ok(self
            .repo
            .epics_in(tx)
            .await?
            .iter()
            .map(Into::into)
            .collect())
    }

    /// Every story with its blockers and pull requests, through the one assembler, all read on this transaction.
    pub async fn export_stories(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<StoryDto>, HandlerError> {
        let stories = self.repo.stories_in(tx).await?;
        self.story_dtos_in(tx, &stories).await
    }

    /// Every sprint, oldest first, with its goals and their links nested, all read on this transaction through the
    /// one `sprint_dto` mapping the sprint routes use.
    pub async fn export_sprints(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<SprintDto>, HandlerError> {
        let mut all = self.repo.sprints_in(tx).await?;
        // `sprints_in` answers newest first (`ORDER BY id DESC`); the export is oldest first.
        all.sort_by_key(|s| s.id);
        let ids: Vec<i64> = all.iter().map(|s| s.id).collect();
        let goals = self.repo.goals_of_in(tx, &ids).await?;
        let goal_ids: Vec<i64> = goals.iter().map(|g| g.id).collect();
        let links = self.repo.links_of_in(tx, &goal_ids).await?;
        Ok(all
            .iter()
            .map(|s| crate::domain::sprint::sprint_dto(s, &goals, &links))
            .collect())
    }

    pub async fn export_decisions(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<RequiredDecisionDto>, HandlerError> {
        Ok(self
            .repo
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
        Ok(self.repo.comments_in(tx).await?)
    }
}
