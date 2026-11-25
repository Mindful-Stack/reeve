//! Which brief a new initiative or epic starts with. The one place the rule is written: the create actions
//! (`create_initiative`, `create_epic`) call `starting_brief` for the brief they hand the planning handler, so
//! every caller of `POST /initiatives` and `POST /epics`, and both MCP create tools, get the same answer. A
//! template is applied only when the caller asks, by name or with `default_template`; a caller that says nothing
//! gets the brief it sent. Plan apply and the import call the planning handlers themselves and never come through
//! here.
use crate::{error::AppError, state::AppState};
use reeve_protocol::{BriefTemplateLevel, ItemType, Key};

/// Where the brief comes from.
#[derive(Debug, PartialEq, Eq)]
enum BriefSource<'a> {
    /// What the caller sent, as sent: its brief, or nothing. What every create got before templates existed.
    AsSent(&'a str),
    /// The level's template with this name.
    Named(&'a str),
    /// The level's default template, or nothing when none is marked.
    LevelDefault,
}

/// The rule, without the database. A blank brief counts as none given. A brief that is not blank together with
/// either request for a template, and the two requests together, are refused: the server never silently drops
/// something the caller named.
fn brief_source<'a>(
    brief: Option<&'a str>,
    template: Option<&'a str>,
    default_template: bool,
) -> Result<BriefSource<'a>, AppError> {
    let written = brief.filter(|b| !b.trim().is_empty());
    match (written, template, default_template) {
        (Some(_), Some(_), _) => Err(AppError::Validation(
            "give a brief or a template, not both: a template only fills a brief that was not given".into(),
        )),
        (Some(_), None, true) => Err(AppError::Validation(
            "give a brief or ask for the default template, not both: a template only fills a brief that was not given"
                .into(),
        )),
        (None, Some(_), true) => Err(AppError::Validation(
            "template and default_template exclude each other: name a template or ask for the level's default"
                .into(),
        )),
        (None, Some(name), false) => Ok(BriefSource::Named(name)),
        (None, None, true) => Ok(BriefSource::LevelDefault),
        // Nothing asked for: the brief as it was sent, blank or not, exactly what the action passed on before.
        (_, None, false) => Ok(BriefSource::AsSent(brief.unwrap_or(""))),
    }
}

/// What a create starts from.
#[derive(Debug)]
pub(super) enum Start {
    /// Create (or replay) with this brief.
    Brief(String),
    /// The create's idempotency key already made this item, and the template it names is gone: answer the item,
    /// `created = false`, without calling the handler.
    Made(Key),
}

/// The brief a new item of `level` is created with, from what its create sent. Nothing about the template is kept:
/// the answer is plain text, and the item is created exactly as if the caller had typed it. With neither `template`
/// nor `default_template` it reads nothing and answers `brief.unwrap_or("")`.
///
/// `idem` is the key as stored (`Acting::write_key`). It is read only when `template` names a template the level
/// has none of: when the key already made an item of this level, that item is the answer (`Start::Made`), so a
/// retry is answered whatever happened to the template since; otherwise the create is refused. An item answered so
/// is answered here and never by the handler with a stand-in brief, so a lookup and a handler that disagreed could
/// not create an item with an empty brief. The lookup is the handler's own (`Handlers::made_under`): what the
/// handler checks before its lookup, the create's `attachments` among it, is checked there too, so a retry the
/// handler would refuse is refused here with the same message whether or not the template still exists. A key that
/// made an item of the other level only is not this level's: the create handlers look a key up in their own level
/// alone, and so does this, so the refusal stands.
pub(super) async fn starting_brief(
    state: &AppState,
    level: BriefTemplateLevel,
    idem: &str,
    attachments: &[uuid::Uuid],
    brief: Option<&str>,
    template: Option<&str>,
    default_template: bool,
) -> Result<Start, AppError> {
    match brief_source(brief, template, default_template)? {
        BriefSource::AsSent(b) => Ok(Start::Brief(b.to_string())),
        BriefSource::Named(name) => {
            if let Some(body) = state
                .handlers
                .brief_template_body(level, Some(name))
                .await?
            {
                return Ok(Start::Brief(body));
            }
            let item = match level {
                BriefTemplateLevel::Initiative => ItemType::Initiative,
                BriefTemplateLevel::Epic => ItemType::Epic,
            };
            match state.handlers.made_under(item, idem, attachments).await? {
                Some(k) => Ok(Start::Made(k)),
                None => Err(AppError::Validation(format!(
                    "no {level} brief template is named {:?}: `reeve template ls --level {level}` lists them",
                    crate::text::echo(name),
                    level = level.wire()
                ))),
            }
        }
        BriefSource::LevelDefault => Ok(Start::Brief(
            state
                .handlers
                .brief_template_body(level, None)
                .await?
                .unwrap_or_default(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_is_applied_only_when_the_caller_asks() {
        use BriefSource::*;
        let ok = |b, t, d| brief_source(b, t, d).unwrap();
        // Nothing asked for: what was sent, as sent, also when it is blank or absent.
        assert_eq!(ok(Some("Mine"), None, false), AsSent("Mine"));
        assert_eq!(ok(Some("  Mine\n"), None, false), AsSent("  Mine\n"));
        assert_eq!(ok(Some(" \n"), None, false), AsSent(" \n"));
        assert_eq!(ok(Some(""), None, false), AsSent(""));
        assert_eq!(ok(None, None, false), AsSent(""));
        // Asked for, with no brief said in each way.
        for none in [None, Some(""), Some(" \n\t")] {
            assert_eq!(ok(none, None, true), LevelDefault, "{none:?}");
            assert_eq!(
                ok(none, Some("Scenario"), false),
                Named("Scenario"),
                "{none:?}"
            );
        }
        // The three contradictions.
        for (b, t, d, says) in [
            (
                Some("Mine"),
                Some("Scenario"),
                false,
                "a brief or a template, not both",
            ),
            (
                Some("Mine"),
                Some("Scenario"),
                true,
                "a brief or a template, not both",
            ),
            (
                Some("Mine"),
                None,
                true,
                "ask for the default template, not both",
            ),
            (None, Some("Scenario"), true, "exclude each other"),
            (Some(" "), Some("Scenario"), true, "exclude each other"),
        ] {
            match brief_source(b, t, d) {
                Err(AppError::Validation(m)) => assert!(m.contains(says), "{b:?} {t:?} {d}: {m}"),
                other => panic!("{b:?} {t:?} {d}: {other:?}"),
            }
        }
    }

    /// A state on a fresh database, with no sign-in provider, and the access policy its callers are built with.
    async fn state() -> (reeve_testdb::TestDb, AppState, reeve_identity::AccessPolicy) {
        use crate::state::{build_state, composed_migrator, pool_options, run_migrations};
        let db = reeve_testdb::database(&composed_migrator()).await;
        let pool = pool_options().connect(&db.url).await.unwrap();
        run_migrations(&pool).await.unwrap();
        let config = crate::config::ServerConfig::parse(&format!(
            r#"database_url = "{}"
key_prefix = "TST"
public_url = "http://127.0.0.1:7474"
allowed_domains = ["example.com"]
admins = ["dan@example.com"]"#,
            db.url
        ))
        .unwrap();
        let policy = reeve_identity::AccessPolicy::floor_only(&config.identity());
        let state = build_state(config, pool).await.unwrap();
        (db, state, policy)
    }

    /// The create actions answer a retry whose named template was removed since with the item its key made, and
    /// say that this call did not create it (`created = false`, what an MCP receipt reports), for both levels and
    /// for an API caller and an MCP caller alike. Catches a refusal on the retry, a second item, and a lookup made
    /// with the key as sent rather than as stored: an MCP caller's key is stored in its own space
    /// (`Acting::write_key`), so looking up the raw key would find nothing and refuse.
    #[tokio::test]
    async fn a_retry_naming_a_removed_template_is_answered_not_created() {
        use crate::actions::{Acting, create_epic, create_initiative};
        use reeve_protocol::{CreateEpic, CreateInitiative, Via};
        let (_db, state, policy) = state().await;
        let now = chrono::Utc::now();
        for client in [None, Some("chat".to_string())] {
            let who = Acting::new(
                "dan@example.com",
                Via::Interactive,
                now,
                &policy,
                client.clone(),
            );
            for level in BriefTemplateLevel::ALL {
                let t = state
                    .handlers
                    .create_brief_template(&who.ctx, level, "Scenario", "# Scenario\n")
                    .await
                    .unwrap();
                let idem = format!("{}-{}", level.wire(), client.is_some());
                let create = || async {
                    match level {
                        BriefTemplateLevel::Initiative => create_initiative(
                            &state,
                            &who,
                            CreateInitiative {
                                idempotency_key: idem.clone(),
                                title: "T".into(),
                                brief: None,
                                horizon: "Now".into(),
                                target_month: None,
                                owner: None,
                                external_ref: None,
                                template: Some("Scenario".into()),
                                default_template: false,
                                attachments: vec![],
                            },
                        )
                        .await
                        .map(|(d, c)| (d.key.to_string(), d.brief, c)),
                        BriefTemplateLevel::Epic => create_epic(
                            &state,
                            &who,
                            CreateEpic {
                                idempotency_key: idem.clone(),
                                title: "T".into(),
                                initiative: None,
                                brief: None,
                                external_ref: None,
                                template: Some("Scenario".into()),
                                default_template: false,
                                attachments: vec![],
                            },
                        )
                        .await
                        .map(|(d, c)| (d.key.to_string(), d.brief, c)),
                    }
                };
                let (key, brief, created) = create().await.unwrap();
                assert_eq!(
                    (brief.as_str(), created),
                    ("# Scenario\n", true),
                    "the control"
                );
                state
                    .handlers
                    .remove_brief_template(&who.ctx, t.id)
                    .await
                    .unwrap();
                let again = create().await.unwrap();
                assert_eq!(
                    again,
                    (key, brief, false),
                    "{level:?} through MCP {}: the first item, not created by the retry",
                    client.is_some()
                );
            }
        }
    }

    /// A retry naming a template removed since is judged as the handler judges every other retry: what the create
    /// handlers check before their own idempotency lookup (at most `MAX_CLAIM` distinct attachment ids) applies
    /// whether or not the template still exists. Catches the shortcut answering a payload the handler's replay
    /// refuses: the same retry is refused with the handler's own message while the template exists (the control)
    /// and after it is gone, and with a valid payload it is still answered the item its key made, for both levels.
    #[tokio::test]
    async fn a_retry_naming_a_removed_template_is_judged_as_the_handler_judges_a_retry() {
        use crate::actions::{Acting, create_epic, create_initiative};
        use reeve_protocol::{CreateEpic, CreateInitiative, Via};
        let (_db, state, policy) = state().await;
        let who = Acting::new(
            "dan@example.com",
            Via::Interactive,
            chrono::Utc::now(),
            &policy,
            None,
        );
        let too_many: Vec<uuid::Uuid> = (0..=reeve_planning::MAX_CLAIM)
            .map(|_| uuid::Uuid::new_v4())
            .collect();
        for level in BriefTemplateLevel::ALL {
            let t = state
                .handlers
                .create_brief_template(&who.ctx, level, "Scenario", "# Scenario\n")
                .await
                .unwrap();
            let idem = format!("judged-{}", level.wire());
            let create = |attachments: Vec<uuid::Uuid>| {
                let (state, who, idem) = (&state, &who, idem.clone());
                async move {
                    match level {
                        BriefTemplateLevel::Initiative => create_initiative(
                            state,
                            who,
                            CreateInitiative {
                                idempotency_key: idem,
                                title: "T".into(),
                                brief: None,
                                horizon: "Now".into(),
                                target_month: None,
                                owner: None,
                                external_ref: None,
                                template: Some("Scenario".into()),
                                default_template: false,
                                attachments,
                            },
                        )
                        .await
                        .map(|(d, c)| (d.key.to_string(), c)),
                        BriefTemplateLevel::Epic => create_epic(
                            state,
                            who,
                            CreateEpic {
                                idempotency_key: idem,
                                title: "T".into(),
                                initiative: None,
                                brief: None,
                                external_ref: None,
                                template: Some("Scenario".into()),
                                default_template: false,
                                attachments,
                            },
                        )
                        .await
                        .map(|(d, c)| (d.key.to_string(), c)),
                    }
                }
            };
            let refused = |r: Result<(String, bool), AppError>| match r {
                Err(AppError::Validation(m)) => m,
                other => panic!("{level:?}: {other:?}"),
            };
            let (key, created) = create(vec![]).await.unwrap();
            assert!(created, "{level:?}: the first create");
            let by_handler = refused(create(too_many.clone()).await);
            assert!(
                by_handler.contains("at most 50 images"),
                "{level:?}: the control: {by_handler}"
            );
            state
                .handlers
                .remove_brief_template(&who.ctx, t.id)
                .await
                .unwrap();
            assert_eq!(
                refused(create(too_many.clone()).await),
                by_handler,
                "{level:?}: the retry after the template is gone"
            );
            assert_eq!(
                create(vec![]).await.unwrap(),
                (key, false),
                "{level:?}: a valid retry is still answered its item"
            );
        }
    }
}
