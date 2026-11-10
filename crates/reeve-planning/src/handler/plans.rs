//! A plan applied in one request (`POST /plans`, `reeve apply`). The whole plan is validated before anything is
//! written: names, references, and each item's own rules (the same constructors the creates use). Then the items are
//! created through the ordinary create handlers, in dependency order, each under the idempotency key
//! `plan:<len(id)>:<id>:<name>`, so a second apply of the same plan answers every item as existing and writes nothing. Nothing
//! here bypasses a rule a single create keeps: activity entries, external references and blocker cycles are checked
//! by the handlers the plan calls. A story carrying a migrated ticket's active state is grandfathered as the
//! importer does: created in that state and logged `imported`, never through `move_story`.
use super::import::check_item_ref;
use super::*;
use crate::domain::items::*;
use reeve_protocol::{
    ApplyPlan, DeliveryState, Kind, PlanApplied, PlannedBlock, PlannedDecision, PlannedItem,
};
use std::collections::HashMap;

/// What a plan reference names: an item the plan creates, or one that exists.
#[derive(Debug, Clone)]
enum Target {
    Planned(ItemType),
    Existing(ItemType, Key),
}

/// The idempotency key of a plan item: the id's length first, so any characters (the skills put `:` in ids, as in
/// their idempotency keys) give one key per plan and name; `id a:b, name c` and `id a, name b:c` differ.
fn plan_key(id: &str, name: &str) -> String {
    format!("plan:{}:{id}:{name}", id.len())
}

fn type_name(t: ItemType) -> &'static str {
    match t {
        ItemType::Initiative => "initiative",
        ItemType::Epic => "epic",
        ItemType::Story => "story",
    }
}

impl Handlers {
    pub async fn apply_plan(&self, c: &Ctx, p: ApplyPlan) -> Result<PlanApplied, HandlerError> {
        let refs = self.validate_plan(&p).await?;
        let id = p.id.trim();
        let idem = |name: &str| plan_key(id, name);
        let mut keys: HashMap<String, Key> = HashMap::new();
        let resolve = |keys: &HashMap<String, Key>, r: &str| -> Key {
            match &refs[r] {
                Target::Planned(_) => keys[r].clone(),
                Target::Existing(_, k) => k.clone(),
            }
        };
        let mut items = vec![];
        for i in &p.initiatives {
            let key = idem(&i.name);
            let (made, created) = self
                .create_initiative_reporting(
                    c,
                    &key,
                    &i.title,
                    i.brief.as_deref().unwrap_or(""),
                    &i.horizon,
                    i.target_month.clone(),
                    i.owner.clone(),
                    i.external_ref.clone(),
                )
                .await?;
            keys.insert(i.name.clone(), made.key().clone());
            items.push(PlannedItem {
                name: i.name.clone(),
                item: ItemType::Initiative,
                key: made.key().clone(),
                created,
            });
        }
        for e in &p.epics {
            let key = idem(&e.name);
            let initiative = e.initiative.as_deref().map(|r| resolve(&keys, r));
            let (made, created) = self
                .create_epic_reporting(
                    c,
                    &key,
                    &e.title,
                    initiative,
                    e.brief.as_deref().unwrap_or(""),
                    e.external_ref.clone(),
                )
                .await?;
            keys.insert(e.name.clone(), made.key().clone());
            items.push(PlannedItem {
                name: e.name.clone(),
                item: ItemType::Epic,
                key: made.key().clone(),
                created,
            });
        }
        for s in &p.stories {
            let key = idem(&s.name);
            let epic = s.epic.as_deref().map(|r| resolve(&keys, r));
            let (made, created) = self
                .create_story_reporting(
                    c,
                    &key,
                    &s.title,
                    s.kind.unwrap_or(Kind::Story),
                    epic,
                    s.description.as_deref().unwrap_or(""),
                    s.criteria.clone(),
                    s.labels.clone(),
                    s.external_ref.clone(),
                    None,
                    s.delivery.unwrap_or(DeliveryState::Todo),
                )
                .await?;
            keys.insert(s.name.clone(), made.key().clone());
            items.push(PlannedItem {
                name: s.name.clone(),
                item: ItemType::Story,
                key: made.key().clone(),
                created,
            });
        }
        let mut decisions = vec![];
        for d in &p.decisions {
            let on = resolve(&keys, &d.on);
            let (made, created) = self
                .require_decision_reporting(
                    c,
                    &idem(&d.name),
                    &on,
                    &d.title,
                    &d.decider,
                    d.due,
                    d.reversibility,
                )
                .await?;
            decisions.push(PlannedDecision {
                name: d.name.clone(),
                id: made.id(),
                created,
            });
        }
        let mut blocks = vec![];
        for b in &p.blocks {
            let story = resolve(&keys, &b.story);
            let blocked_by = resolve(&keys, &b.blocked_by);
            let created = self.add_block_reporting(c, &story, &blocked_by).await?;
            blocks.push(PlannedBlock {
                story,
                blocked_by,
                created,
            });
        }
        Ok(PlanApplied {
            items,
            decisions,
            blocks,
        })
    }

    /// Every problem in the plan, all at once (an agent fixes the file in one pass), before anything is written. Returns
    /// what each reference in the plan names.
    async fn validate_plan(&self, p: &ApplyPlan) -> Result<HashMap<String, Target>, HandlerError> {
        let mut errors: Vec<String> = vec![];
        let id = p.id.trim();
        if id.is_empty() || id.chars().count() > 100 {
            errors.push("the plan needs an id of 1..=100 characters".into());
        }
        let prefix = &self.settings.key_prefix;
        let probe_key = Key::new(prefix, 1).expect("prefix validated by config");

        // Names: unique across every list, and never key-shaped, so a reference is never ambiguous.
        let mut refs: HashMap<String, Target> = HashMap::new();
        let named = p
            .initiatives
            .iter()
            .map(|i| (&i.name, ItemType::Initiative))
            .chain(p.epics.iter().map(|e| (&e.name, ItemType::Epic)))
            .chain(p.stories.iter().map(|s| (&s.name, ItemType::Story)));
        for (name, t) in named {
            if name.trim().is_empty() || name.trim() != name {
                errors.push(format!(
                    "{} name {name:?} must be non-empty with no surrounding spaces",
                    type_name(t)
                ));
            } else if name.parse::<reeve_protocol::KeyRef>().is_ok() {
                errors.push(format!(
                    "{} name {name:?} looks like a key; a name must not, so a reference is never ambiguous",
                    type_name(t)
                ));
            } else if refs.insert(name.clone(), Target::Planned(t)).is_some() {
                errors.push(format!(
                    "{name:?} is used twice; every name in a plan names one item"
                ));
            }
        }
        let mut decision_names = std::collections::HashSet::new();
        for d in &p.decisions {
            if refs.contains_key(&d.name) || !decision_names.insert(&d.name) {
                errors.push(format!(
                    "{:?} is used twice; every name in a plan names one item",
                    d.name
                ));
            }
        }

        // References, checked once every item's own rules are: (the reference, the type it must name, where it is).
        let mut pending: Vec<(String, Option<ItemType>, String)> = vec![];

        for i in &p.initiatives {
            let at = format!("initiative {:?}", i.name);
            if let Err(e) = Initiative::new(
                probe_key.clone(),
                &i.title,
                i.brief.as_deref().unwrap_or(""),
                &i.horizon,
                &self.settings.horizon_buckets,
                i.target_month.clone(),
                i.owner.clone(),
            ) {
                errors.push(format!("{at}: {e}"));
            }
            if let Some(r) = &i.external_ref
                && let Err(HandlerError::Validation(m)) = check_item_ref(r, prefix)
            {
                errors.push(format!("{at}: {m}"));
            }
        }
        for e in &p.epics {
            let at = format!("epic {:?}", e.name);
            if let Err(err) = Epic::new(
                probe_key.clone(),
                &e.title,
                None,
                e.brief.as_deref().unwrap_or(""),
            ) {
                errors.push(format!("{at}: {err}"));
            }
            if let Some(r) = &e.external_ref
                && let Err(HandlerError::Validation(m)) = check_item_ref(r, prefix)
            {
                errors.push(format!("{at}: {m}"));
            }
            if let Some(r) = &e.initiative {
                pending.push((
                    r.clone(),
                    Some(ItemType::Initiative),
                    format!("{at}: initiative {r:?}"),
                ));
            }
        }
        for s in &p.stories {
            let at = format!("story {:?}", s.name);
            if let Err(err) = Story::new(
                probe_key.clone(),
                &s.title,
                s.kind.unwrap_or(Kind::Story),
                None,
                s.description.as_deref().unwrap_or(""),
                s.criteria.clone(),
                None,
                s.labels.clone(),
            ) {
                errors.push(format!("{at}: {err}"));
            }
            if let Some(r) = &s.external_ref
                && let Err(HandlerError::Validation(m)) = check_item_ref(r, prefix)
            {
                errors.push(format!("{at}: {m}"));
            }
            if s.delivery.is_some() && s.external_ref.is_none() {
                errors.push(format!(
                    "{at}: a delivery state requires an external_ref; only migrated work skips the gate"
                ));
            }
            if s.delivery == Some(DeliveryState::Done) {
                errors.push(format!(
                    "{at}: delivery done is the importer's (it needs a resolution); a plan takes todo, in_progress, in_review or testing"
                ));
            }
            if let Some(r) = &s.epic {
                pending.push((r.clone(), Some(ItemType::Epic), format!("{at}: epic {r:?}")));
            }
        }
        for d in &p.decisions {
            let at = format!("decision {:?}", d.name);
            if d.title.trim().is_empty() || d.decider.trim().is_empty() {
                errors.push(format!("{at}: title and decider are required"));
            }
            pending.push((d.on.clone(), None, format!("{at}: on {:?}", d.on)));
        }
        for b in &p.blocks {
            let at = format!("block {:?} by {:?}", b.story, b.blocked_by);
            if b.story == b.blocked_by {
                errors.push(format!("{at}: a story cannot block itself"));
            }
            pending.push((
                b.story.clone(),
                Some(ItemType::Story),
                format!("{at}: story {:?}", b.story),
            ));
            pending.push((
                b.blocked_by.clone(),
                Some(ItemType::Story),
                format!("{at}: story {:?}", b.blocked_by),
            ));
        }

        for (r, wanted, at) in pending {
            match refs.get(&r) {
                Some(Target::Planned(t)) | Some(Target::Existing(t, _)) => {
                    if wanted.is_some_and(|w| w != *t) {
                        errors.push(format!(
                            "{at} is a {} in the plan, not a {}",
                            type_name(*t),
                            type_name(wanted.unwrap())
                        ));
                    }
                }
                None => match self.existing(&r, wanted).await? {
                    Some((t, k)) => {
                        refs.insert(r, Target::Existing(t, k));
                    }
                    None => errors.push(match wanted {
                        Some(w) => format!(
                            "{at} is neither a name in the plan nor an existing {}",
                            type_name(w)
                        ),
                        None => format!("{at} is neither a name in the plan nor an existing item"),
                    }),
                },
            }
        }
        self.check_external_refs(p, &mut errors).await?;
        if errors.is_empty() {
            Ok(refs)
        } else {
            Err(HandlerError::Validation(errors.join("; ")))
        }
    }

    /// A reference names one item across all three types: no two items in the plan may give the same one, and one
    /// already held by an item is refused unless that item is this plan's own (a second apply of the same plan).
    async fn check_external_refs(
        &self,
        p: &ApplyPlan,
        errors: &mut Vec<String>,
    ) -> Result<(), HandlerError> {
        let id = p.id.trim();
        let given = p
            .initiatives
            .iter()
            .map(|i| (&i.name, ItemType::Initiative, &i.external_ref))
            .chain(
                p.epics
                    .iter()
                    .map(|e| (&e.name, ItemType::Epic, &e.external_ref)),
            )
            .chain(
                p.stories
                    .iter()
                    .map(|s| (&s.name, ItemType::Story, &s.external_ref)),
            );
        let mut seen: HashMap<String, &String> = HashMap::new();
        for (name, t, r) in given {
            let Some(r) = r.as_deref().map(str::trim) else {
                continue;
            };
            let at = format!("{} {name:?}", type_name(t));
            if let Some(first) = seen.insert(r.to_string(), name) {
                errors.push(format!(
                    "{at}: {r} is also the external reference of {first:?} in the plan"
                ));
                continue;
            }
            let mine = self
                .ports
                .keys
                .find_by_idempotency(t, &plan_key(id, name))
                .await?;
            for holder in [ItemType::Initiative, ItemType::Epic, ItemType::Story] {
                if let Some(k) = self.ports.keys.find_by_external_ref(holder, r).await?
                    && !(holder == t && mine.as_ref() == Some(&k))
                {
                    errors.push(format!(
                        "{at}: {r} is already the external reference of {} {k}",
                        type_name(holder)
                    ));
                }
            }
        }
        Ok(())
    }

    /// The existing item a reference names, of the wanted type (any type for a decision's target), or None.
    async fn existing(
        &self,
        r: &str,
        wanted: Option<ItemType>,
    ) -> Result<Option<(ItemType, Key)>, HandlerError> {
        let found = match wanted {
            Some(t) => self.resolve_ref(r, t).await.map(|k| (t, k)),
            None => self.resolve_any(r).await,
        };
        let (t, k) = match found {
            Ok(v) => v,
            Err(HandlerError::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        // A key of the current prefix resolves as is; the item must also exist.
        match self.item_type_of(&k).await {
            Ok(actual) if actual == t => Ok(Some((t, k))),
            Ok(_) | Err(HandlerError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
