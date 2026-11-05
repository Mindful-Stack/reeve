use crate::items::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// `reeve story context`: everything an agent needs before starting a story.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct StoryContextDto {
    pub story: StoryDto,
    pub epic: Option<EpicDto>,
    pub initiative: Option<InitiativeDto>,
    pub required_decisions: Vec<RequiredDecisionDto>,
    pub blockers: Vec<StoryDto>,
    pub comments: Vec<CommentDto>,
    pub readiness: crate::readiness::Readiness,
    /// The active sprint's goals the story serves, with the sprint's dates and `via`, from the same snapshot. Always
    /// sent; JSON without it (an older server) reads as none.
    #[serde(default)]
    #[schema(required = true)]
    pub goals: Vec<crate::sprints::GoalContextDto>,
}
