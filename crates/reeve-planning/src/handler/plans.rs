//! A plan applied in one request (`POST /plans`, `reeve apply`). The whole plan is validated before anything is
//! written: names, references, and each item's own rules (the same constructors the creates use). Then the items are
//! created through the plan-only create entries (`create_*_guarded`, which share every check and the insert with the
//! ordinary create handlers), in dependency order, each under the idempotency key
//! `plan:<len(id)>:<id>:<name>`, so a second apply of the same plan answers every item as existing and writes nothing. Nothing
//! here bypasses a rule a single create keeps: activity entries, external references and blocker targets are checked
//! by the handlers the plan calls. A story carrying a migrated ticket's active state is grandfathered as the
//! importer does: created in that state and logged `imported`, never through `move_story`.
//!
//! A plan name is one item, whichever list it stands in. The check refuses a name that an earlier apply made at
//! another level (`check_name_levels`). Two applies of one plan id that run at once can both pass that check, so
//! each create a plan makes (`create_initiative_guarded` and its epic and story forms) also takes a lock on its
//! idempotency key and looks for the key at the other two levels inside its transaction; the apply that comes
//! second is refused there (`name_taken_meanwhile`). That covers applies of one plan id: a single create that
//! sends a plan's key by hand takes no lock and is not stopped (GRA-984 reserves the prefix). A plan is not
//! stored: nothing reads a plan's state and it has no lifecycle, so the rule is guarded where an external
//! reference is, by a lock and a look in the creating transaction.
//!
//! Each create commits on its own: a plan has no transaction around its items. Its item links and web links
//! (kb/ADR-0031) are written last, once every item it names exists, each in a transaction of its own, through the
//! transaction-taking forms of the item link and web link handlers, which record the entry on every item named; the
//! ports' writers are never named here (the isolation lint refuses them). One transaction per link, never one for
//! all: an activity entry takes the activity log's lock until its transaction ends, so nothing may be inserted
//! after it in the same transaction. Both are idempotent by their content, so a second apply answers each as
//! existing, and finishes a plan whose earlier apply stopped at a link, once whatever made that link fail is
//! corrected. The links the plan's items have already are read once before the link phase, and one found there
//! is answered without a transaction (`apply_plan_links`).
use super::import::check_item_ref;
use super::item_links::ResolvedItemLink;
use super::web_links::repeat_among;
use super::*;
use crate::domain::item_link::{ItemLink, ItemLinkError};
use crate::domain::items::*;
use crate::domain::web_link::{WebLink, WebLinkError};
use crate::ports::StoredWebLink;
use reeve_protocol::{
    ApplyPlan, DeliveryState, Kind, MAX_PLAN_ITEM_LINKS, MAX_PLAN_WEB_LINKS, PlanApplied, PlanLink,
    PlanWebLink, PlannedBlock, PlannedDecision, PlannedItem, PlannedLink, PlannedWebLink,
};
use std::collections::{HashMap, HashSet};

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

/// What every reference of a plan names, kept by the place a reference stands in. A plan name is one item
/// wherever it stands (names are unique and never key-shaped). Anything else is an existing item, and which one
/// depends on what the place takes: a place that takes one level (a story's `epic`, an epic's `initiative`, a
/// blocker's two stories) is resolved as `resolve_ref` does for that level, and a place that takes an item of any
/// level (a decision's `on`, an item link's ends, a web link's `on`) as `resolve_any` does. The two can differ for
/// one spelling: `OLD-1` may be the native key of an epic and the external reference of a story. So an answer is
/// kept under the scope it was asked in and never served to the other.
#[derive(Debug, Default)]
struct Refs {
    planned: HashMap<String, ItemType>,
    /// (the level, the reference) -> the existing item of that level.
    typed: HashMap<(ItemType, String), Key>,
    /// The reference -> the existing item of any level.
    any: HashMap<String, (ItemType, Key)>,
}

impl Refs {
    /// What `r` names where an item of level `wanted` stands (any level when None); None when nothing has been
    /// resolved for it in that scope.
    fn get(&self, r: &str, wanted: Option<ItemType>) -> Option<Target> {
        if let Some(t) = self.planned.get(r) {
            return Some(Target::Planned(*t));
        }
        match wanted {
            Some(t) => self
                .typed
                .get(&(t, r.to_owned()))
                .map(|k| Target::Existing(t, k.clone())),
            None => self
                .any
                .get(r)
                .map(|(t, k)| Target::Existing(*t, k.clone())),
        }
    }
    fn keep(&mut self, r: String, wanted: Option<ItemType>, found: (ItemType, Key)) {
        match wanted {
            Some(t) => {
                self.typed.insert((t, r), found.1);
            }
            None => {
                self.any.insert(r, found);
            }
        }
    }
    /// The level and key `r` came to in the scope `wanted`, once the plan's items exist: `keys` holds the key of
    /// every item the plan names, and an existing item was resolved by the validation in that same scope.
    fn end(
        &self,
        keys: &HashMap<String, Key>,
        r: &str,
        wanted: Option<ItemType>,
    ) -> (ItemType, Key) {
        match self.get(r, wanted) {
            Some(Target::Planned(t)) => (t, keys[r].clone()),
            Some(Target::Existing(t, k)) => (t, k),
            None => {
                unreachable!("the validation resolved every reference of the plan in its scope")
            }
        }
    }
}

fn type_name(t: ItemType) -> &'static str {
    match t {
        ItemType::Initiative => "initiative",
        ItemType::Epic => "epic",
        ItemType::Story => "story",
    }
}

/// The refusal of an apply whose create found the plan name `name` already an item of another level (`level`,
/// `key`). That only happens when another apply of this plan id created the item after this apply's check
/// (`check_name_levels`) passed, so this apply has stopped part-way. It does not open as the check's message does,
/// which means nothing was written, and it claims only what is always true: items are written initiatives first,
/// then epics, then stories; some before this one may exist, written by this apply or by the other; this apply
/// wrote nothing after it, and no decisions, blockers or links (the other apply may have written all of them).
fn name_taken_meanwhile(name: &str, level: ItemType, key: &Key) -> HandlerError {
    let a_level = match level {
        ItemType::Initiative => "an initiative",
        ItemType::Epic => "an epic",
        ItemType::Story => "a story",
    };
    HandlerError::Validation(format!(
        "{name:?} is {a_level} ({key}), created by another apply of this plan while this one ran; a name keeps \
         its level. This apply stopped at {name:?}: items written before it (initiatives, then epics, then \
         stories) may exist; this apply wrote nothing after it, and no decisions, blockers or links. List \
         {name:?} as {a_level}, or give it a new name, and apply the same id again; what exists is reused."
    ))
}

/// Where a problem with an item link is, as the plan wrote it. "item link", never bare "link" (kb/ADR-0031 R6):
/// the same answer may name a plan's blockers and a decision's record link.
fn link_at(l: &PlanLink) -> String {
    format!("item link {:?} {} {:?}", l.from, l.link_type.wire(), l.to)
}

/// Where a problem with a web link is: its title (the first 40 characters) and the item it is on. Never the URL:
/// one refused for carrying a user name or a password must not come back in an error message.
fn web_link_at(w: &PlanWebLink) -> String {
    let title: String = w.title.trim().chars().take(40).collect();
    format!("web link {title:?} on {:?}", w.on)
}

/// Which item a reference of a plan names, as one comparable value. `Key`: the item exists, either an existing
/// item (named by its key or its external reference) or a plan item an earlier apply of this plan created. `Name`:
/// a plan item that does not exist yet; plan names are unique, so the name is its identity until it has a key.
/// A `Key` and a `Name` are never one item: an item without a key cannot be one that has one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Identity {
    Key(Key),
    Name(String),
}

impl Handlers {
    pub async fn apply_plan(&self, c: &Ctx, p: ApplyPlan) -> Result<PlanApplied, HandlerError> {
        let refs = self.validate_plan(&p).await?;
        let id = p.id.trim();
        let idem = |name: &str| plan_key(id, name);
        let mut keys: HashMap<String, Key> = HashMap::new();
        // Each reference in the scope the validation resolved it in (`Refs`).
        let resolve = |keys: &HashMap<String, Key>, r: &str, wanted: Option<ItemType>| -> Key {
            refs.end(keys, r, wanted).1
        };
        let mut items = vec![];
        for i in &p.initiatives {
            let key = idem(&i.name);
            let (made, created) = match self
                .create_initiative_guarded(
                    c,
                    &key,
                    &i.title,
                    i.brief.as_deref().unwrap_or(""),
                    &i.horizon,
                    i.target_month.clone(),
                    i.owner.clone(),
                    i.external_ref.clone(),
                    &[],
                )
                .await?
            {
                Guarded::Item { item, created } => (item, created),
                Guarded::HeldElsewhere { level, key: held } => {
                    return Err(name_taken_meanwhile(&i.name, level, &held));
                }
            };
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
            let initiative = e
                .initiative
                .as_deref()
                .map(|r| resolve(&keys, r, Some(ItemType::Initiative)));
            let (made, created) = match self
                .create_epic_guarded(
                    c,
                    &key,
                    &e.title,
                    initiative,
                    e.brief.as_deref().unwrap_or(""),
                    e.external_ref.clone(),
                    &[],
                )
                .await?
            {
                Guarded::Item { item, created } => (item, created),
                Guarded::HeldElsewhere { level, key: held } => {
                    return Err(name_taken_meanwhile(&e.name, level, &held));
                }
            };
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
            let epic = s
                .epic
                .as_deref()
                .map(|r| resolve(&keys, r, Some(ItemType::Epic)));
            let (made, created) = match self
                .create_story_guarded(
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
                    &[],
                )
                .await?
            {
                Guarded::Item { item, created } => (item, created),
                Guarded::HeldElsewhere { level, key: held } => {
                    return Err(name_taken_meanwhile(&s.name, level, &held));
                }
            };
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
            let on = resolve(&keys, &d.on, None);
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
            let story = resolve(&keys, &b.story, Some(ItemType::Story));
            let blocked_by = resolve(&keys, &b.blocked_by, Some(ItemType::Story));
            let created = self.add_block_reporting(c, &story, &blocked_by).await?;
            blocks.push(PlannedBlock {
                story,
                blocked_by,
                created,
            });
        }
        let (links, web_links) = self.apply_plan_links(c, &p, &refs, &keys).await?;
        Ok(PlanApplied {
            items,
            decisions,
            blocks,
            links,
            web_links,
        })
    }

    /// The plan's item links and web links, written after its items exist. The item links and web links the
    /// plan's items have today are read once first (`plan_links_today`); an entry found there is already stored and
    /// answers `created` false with no transaction at all, so a replay of a large plan costs two reads and not a
    /// transaction per entry. An entry not found there is written in a transaction of its own (begin, the `_in`
    /// form, commit), as the plan's blockers commit one by one: a link's row and its entries commit together or
    /// not at all. The `_in` form is idempotent itself, so an entry someone else wrote after the read, or one the
    /// plan lists twice, still answers `created` false and writes nothing. Never one transaction for all of them:
    /// an entry takes the activity log's lock until its transaction ends, and a transaction that went on to
    /// insert the next link while holding it could deadlock with a caller adding that same link. A link that
    /// fails leaves the earlier ones in; applying the same plan id again finishes, once whatever made the link
    /// fail is corrected. The ends are the plan's own (the level from the validation, the key from the item just
    /// created or the existing one), so no item is looked up here.
    async fn apply_plan_links(
        &self,
        c: &Ctx,
        p: &ApplyPlan,
        refs: &Refs,
        keys: &HashMap<String, Key>,
    ) -> Result<(Vec<PlannedLink>, Vec<PlannedWebLink>), HandlerError> {
        // The ends as the plan named them; `ResolvedItemLink` keeps each level with its key when a `relates_to`
        // is stored with the lower key first.
        let mut wanted_links = Vec::with_capacity(p.links.len());
        for l in &p.links {
            wanted_links.push(ResolvedItemLink::new(
                refs.end(keys, &l.from, None),
                l.link_type,
                refs.end(keys, &l.to, None),
            )?);
        }
        let mut wanted_web = Vec::with_capacity(p.web_links.len());
        for w in &p.web_links {
            let (level, on) = refs.end(keys, &w.on, None);
            wanted_web.push((level, on, WebLink::new(&w.title, &w.url)?));
        }
        let (stored_links, stored_web) = self.plan_links_today(&wanted_links, &wanted_web).await?;

        let mut links = Vec::with_capacity(wanted_links.len());
        for link in &wanted_links {
            let created = if stored_links.contains(link.link()) {
                false
            } else {
                let mut tx = self.pool.begin().await?;
                let created = self.add_item_link_in(&mut tx, c, link).await?;
                tx.commit().await?;
                created
            };
            let [(from_level, from), (to_level, to)] = link.ends();
            links.push(PlannedLink {
                from: from.clone(),
                from_level,
                link_type: link.link().link_type(),
                to: to.clone(),
                to_level,
                created,
            });
        }
        let mut web_links = Vec::with_capacity(wanted_web.len());
        for (level, on, link) in wanted_web {
            // The item's web links as read; a row this one repeats (`repeat_among`) has its id. Anything else, a
            // refusal included, is the `_in` form's to answer on what is stored by then.
            let on_the_item = stored_web.get(&on).map(Vec::as_slice).unwrap_or_default();
            let stored = match repeat_among(&link, on_the_item) {
                Ok(Some(row)) => Some(row.id),
                _ => None,
            };
            let (id, created) = match stored {
                Some(id) => (id, false),
                None => {
                    let mut tx = self.pool.begin().await?;
                    let (added, created) =
                        self.add_web_link_in(&mut tx, c, level, &on, &link).await?;
                    tx.commit().await?;
                    (added.id, created)
                }
            };
            web_links.push(PlannedWebLink {
                on,
                level,
                id,
                title: link.title().to_owned(),
                url: link.url().to_owned(),
                created,
            });
        }
        Ok((links, web_links))
    }

    /// The item links and the web links that the items named by the plan's links have today, in one read-only
    /// snapshot and one statement each, through the ports' reads (the ones a listing uses): every item link with
    /// one of those items at either end, and every web link by its item. Empty lists send nothing.
    async fn plan_links_today(
        &self,
        links: &[ResolvedItemLink],
        web: &[(ItemType, Key, WebLink)],
    ) -> Result<(HashSet<ItemLink>, HashMap<Key, Vec<StoredWebLink>>), HandlerError> {
        // One end of each item link is enough: the read answers every link with a listed item at either end.
        let link_items: Vec<Key> = links
            .iter()
            .map(|l| l.link().from_key().clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let web_items: Vec<Key> = web
            .iter()
            .map(|(_, on, _)| on.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if link_items.is_empty() && web_items.is_empty() {
            return Ok((HashSet::new(), HashMap::new()));
        }
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let read = async {
            let stored_links = if link_items.is_empty() {
                vec![]
            } else {
                self.ports
                    .item_links
                    .item_links_of_in(&mut tx, &link_items)
                    .await?
            };
            let stored_web = if web_items.is_empty() {
                vec![]
            } else {
                self.ports
                    .web_links
                    .web_links_of_in(&mut tx, &web_items)
                    .await?
            };
            Ok::<_, HandlerError>((stored_links, stored_web))
        }
        .await;
        let (stored_links, stored_web) = end_read(tx, read).await?;
        let mut by_item: HashMap<Key, Vec<StoredWebLink>> = HashMap::new();
        for w in stored_web {
            by_item.entry(w.item.clone()).or_default().push(w);
        }
        Ok((stored_links.into_iter().collect(), by_item))
    }

    /// Every problem in the plan, all at once (an agent fixes the file in one pass), before anything is written. Returns
    /// what each reference in the plan names, in the scope of the place it stands in.
    async fn validate_plan(&self, p: &ApplyPlan) -> Result<Refs, HandlerError> {
        let mut errors: Vec<String> = vec![];
        let id = p.id.trim();
        if id.is_empty() || id.chars().count() > 100 {
            errors.push("the plan needs an id of 1..=100 characters".into());
        }
        let prefix = &self.settings.key_prefix;
        let probe_key = Key::new(prefix, 1).expect("prefix validated by config");

        // Names: unique across every list, and never key-shaped, so a reference is never ambiguous.
        let mut refs = Refs::default();
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
            } else if refs.planned.insert(name.clone(), t).is_some() {
                errors.push(format!(
                    "{name:?} is used twice; every name in a plan names one item"
                ));
            }
        }
        self.check_name_levels(p, &refs, &mut errors).await?;
        let mut decision_names = HashSet::new();
        for d in &p.decisions {
            if refs.planned.contains_key(&d.name) || !decision_names.insert(&d.name) {
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
            // A plan names its initiatives' months as a request does, so the rule for a named month applies
            // (`named_target_month`). The owner is not judged here: who is allowed is identity's to say, and
            // planning cannot ask.
            if let Err(e) = named_target_month(i.target_month.clone()) {
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

        // A plan is one request, answered within a client's timeout: the two lists are bounded. An oversize list
        // is still checked entry by entry, so its other problems come back in the same answer.
        if p.links.len() > MAX_PLAN_ITEM_LINKS {
            errors.push(format!(
                "a plan holds at most {MAX_PLAN_ITEM_LINKS} item links; this one has {}",
                p.links.len()
            ));
        }
        if p.web_links.len() > MAX_PLAN_WEB_LINKS {
            errors.push(format!(
                "a plan holds at most {MAX_PLAN_WEB_LINKS} web links; this one has {}",
                p.web_links.len()
            ));
        }

        // Item links (kb/ADR-0031): each end is an item of any level, resolved as a decision's `on` is and as
        // `reeve link add` resolves it (`resolve_any`), never by an answer a typed place got for the same spelling.
        for l in &p.links {
            let at = link_at(l);
            pending.push((l.from.clone(), None, format!("{at}: from {:?}", l.from)));
            pending.push((l.to.clone(), None, format!("{at}: to {:?}", l.to)));
        }

        // Web links: the title and URL by the constructor the handler uses; `on` is an item of any level. An entry
        // that fails here is None in `web` and takes no part in the checks on URLs below.
        let mut web: Vec<Option<WebLink>> = Vec::with_capacity(p.web_links.len());
        for w in &p.web_links {
            let at = web_link_at(w);
            match WebLink::new(&w.title, &w.url) {
                Ok(link) => web.push(Some(link)),
                Err(e) => {
                    errors.push(format!("{at}: {e}"));
                    web.push(None);
                }
            }
            pending.push((w.on.clone(), None, format!("{at}: on {:?}", w.on)));
        }

        for (r, wanted, at) in pending {
            match refs.get(&r, wanted) {
                Some(Target::Planned(t)) => {
                    if let Some(w) = wanted.filter(|w| *w != t) {
                        errors.push(format!(
                            "{at} is a {} in the plan, not a {}",
                            type_name(t),
                            type_name(w)
                        ));
                    }
                }
                // Resolved in this scope already.
                Some(Target::Existing(..)) => {}
                None => match self.existing(&r, wanted).await? {
                    Some(found) => refs.keep(r, wanted, found),
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
        // Checked once the references are resolved, on what each end is and not on how it is spelled: an item
        // named by its key at one end and by its external reference, or by the name an earlier apply of this plan
        // made it under, at the other is one item.
        let ids = self.plan_identities(p, &refs).await?;
        for l in &p.links {
            let (Some(from), Some(to)) = (ids.get(&l.from), ids.get(&l.to)) else {
                continue;
            };
            let refused = match (from, to) {
                // Both items exist: the rule is the domain's, on the keys the link will be written with.
                (Identity::Key(a), Identity::Key(b)) => {
                    ItemLink::new(a.clone(), l.link_type, b.clone()).err()
                }
                // Neither exists yet: one name is one item.
                (Identity::Name(a), Identity::Name(b)) => {
                    (a == b).then_some(ItemLinkError::SelfLink)
                }
                // One exists and the other does not: two items.
                _ => None,
            };
            if let Some(e) = refused {
                errors.push(format!("{}: {e}", link_at(l)));
            }
        }
        self.check_plan_web_links(p, &ids, &web, &mut errors)
            .await?;
        self.check_external_refs(p, &mut errors).await?;
        if errors.is_empty() {
            Ok(refs)
        } else {
            Err(HandlerError::Validation(errors.join("; ")))
        }
    }

    /// A name keeps its level across applies of one plan id. An item is found again under its level, its plan id
    /// and its name (`plan_key`), so a name listed under another level than an earlier apply made it at would
    /// miss that item, create a second one, and take the plan's links and decisions with it. Refused here, before
    /// anything is written, naming the level the name had and the key it has.
    async fn check_name_levels(
        &self,
        p: &ApplyPlan,
        refs: &Refs,
        errors: &mut Vec<String>,
    ) -> Result<(), HandlerError> {
        const LEVELS: [ItemType; 3] = [ItemType::Initiative, ItemType::Epic, ItemType::Story];
        let id = p.id.trim();
        let named = p
            .initiatives
            .iter()
            .map(|i| (&i.name, ItemType::Initiative))
            .chain(p.epics.iter().map(|e| (&e.name, ItemType::Epic)))
            .chain(p.stories.iter().map(|s| (&s.name, ItemType::Story)));
        for (name, t) in named {
            // A name refused above (empty, key-shaped, used twice under two levels) has its own message.
            if refs.planned.get(name) != Some(&t) {
                continue;
            }
            for earlier in LEVELS.into_iter().filter(|l| *l != t) {
                if let Some(k) = self
                    .ports
                    .keys
                    .find_by_idempotency(earlier, &plan_key(id, name))
                    .await?
                {
                    let article = if earlier == ItemType::Story {
                        "a"
                    } else {
                        "an"
                    };
                    errors.push(format!(
                        "{name:?} was {article} {} ({k}) in an earlier apply of this plan; a name keeps its level",
                        type_name(earlier)
                    ));
                }
            }
        }
        Ok(())
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

    /// The identity of every reference the plan's item links and web links name, resolved once. A key or an external reference is
    /// the key it resolved to; a plan name is the key an earlier apply of this plan gave it (the lookup
    /// `check_external_refs` makes, through the item's idempotency key), or the name itself when there is none. A
    /// reference that names nothing has no entry: the check has reported it already.
    async fn plan_identities(
        &self,
        p: &ApplyPlan,
        refs: &Refs,
    ) -> Result<HashMap<String, Identity>, HandlerError> {
        let id = p.id.trim();
        let named = p
            .links
            .iter()
            .flat_map(|l| [&l.from, &l.to])
            .chain(p.web_links.iter().map(|w| &w.on));
        let mut ids: HashMap<String, Identity> = HashMap::new();
        for r in named {
            if ids.contains_key(r) {
                continue;
            }
            let identity = match refs.get(r, None) {
                None => continue,
                Some(Target::Existing(_, k)) => Identity::Key(k),
                Some(Target::Planned(t)) => {
                    match self
                        .ports
                        .keys
                        .find_by_idempotency(t, &plan_key(id, r))
                        .await?
                    {
                        Some(k) => Identity::Key(k),
                        None => Identity::Name(r.clone()),
                    }
                }
            };
            ids.insert(r.clone(), identity);
        }
        Ok(ids)
    }

    /// An item has a URL once (spec section 1): the same URL under the same title is a repeat, under another title
    /// an error. Checked here, before anything is written, in two places. Within the plan: the first title a URL
    /// gets on an item is the one a later entry must repeat. Against what is stored: the web links the item has
    /// today. Both ask `WebLink::is_new_among`, the one comparison there is (the web link handler's too). "The item" is the entry's
    /// identity (`ids`), not its spelling: an existing item whichever way it is named, and for a plan name the item
    /// an earlier apply of this plan created, or the name itself while there is none.
    async fn check_plan_web_links(
        &self,
        p: &ApplyPlan,
        ids: &HashMap<String, Identity>,
        web: &[Option<WebLink>],
        errors: &mut Vec<String>,
    ) -> Result<(), HandlerError> {
        // The item -> the web links the plan's earlier entries give it, so an entry is compared with its own item's
        // earlier entries and no others. `WebLink::is_new_among` decides, as it does against what is stored; only
        // the wording is the plan's own, since nothing is "already" on the item.
        let mut earlier: HashMap<&Identity, Vec<&WebLink>> = HashMap::new();
        // (where, the item's key today, the link) for every entry whose item exists already.
        let mut stored_checks: Vec<(String, Key, &WebLink)> = vec![];
        for (w, link) in p.web_links.iter().zip(web) {
            let (Some(link), Some(who)) = (link.as_ref(), ids.get(&w.on)) else {
                continue;
            };
            let at = web_link_at(w);
            let on_the_item = earlier.entry(who).or_default();
            match link.is_new_among(on_the_item.iter().copied()) {
                Ok(true) => on_the_item.push(link),
                Ok(false) => {}
                Err(WebLinkError::SameUrlOtherTitle { titled }) => errors.push(format!(
                    "{at}: the plan gives {:?} that URL under two titles ({titled:?} and {:?}); an item has a URL once",
                    w.on,
                    link.title()
                )),
                // `is_new_among` makes no other error; were it to, the plan is refused with its words.
                Err(e) => errors.push(format!("{at}: {e}")),
            }
            if let Identity::Key(key) = who {
                stored_checks.push((at, key.clone(), link));
            }
        }
        if stored_checks.is_empty() {
            return Ok(());
        }
        let mut keys: Vec<Key> = stored_checks.iter().map(|(_, k, _)| k.clone()).collect();
        keys.sort_unstable_by_key(|k| k.to_string());
        keys.dedup();
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let read = self
            .ports
            .web_links
            .web_links_of_in(&mut tx, &keys)
            .await
            .map_err(HandlerError::from);
        let stored = end_read(tx, read).await?;
        // The stored rows by item, grouped once: each entry is compared with its own item's web links only, which
        // is also what `is_new_among` takes (the web links of one item).
        let mut by_item: HashMap<&Key, Vec<&WebLink>> = HashMap::new();
        for s in &stored {
            by_item.entry(&s.item).or_default().push(&s.link);
        }
        for (at, key, link) in stored_checks {
            let of_item = by_item.get(&key).map(Vec::as_slice).unwrap_or_default();
            if let Err(e) = link.is_new_among(of_item.iter().copied()) {
                errors.push(format!("{at}: {e}"));
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
