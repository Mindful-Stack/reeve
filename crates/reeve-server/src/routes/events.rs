//! `GET /api/v1/events`: server-sent activity, by polling the committed activity log every `[events] poll_ms`
//! (default two seconds). No in-process bus, so publishing after commit (kb/ADR-0010 R5) holds trivially and a second
//! instance would also work.
//! Ids become visible in id order because the activity writer holds an advisory lock from the insert to commit.
use crate::{auth::Caller, error::AppError, extract::ApiQuery, state::AppState};
use axum::{
    extract::State,
    http::HeaderMap,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use chrono::Utc;
use futures_util::StreamExt;
use reeve_activity::ActivityFilter;
use serde::Deserialize;
use std::{convert::Infallible, time::Duration};
use utoipa_axum::{router::OpenApiRouter, routes};

const KEEP_ALIVE: Duration = Duration::from_secs(15);
const BATCH: i64 = 500;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EventsQuery {
    /// Resume after this entry id; `Last-Event-ID` is also honoured and wins when both are present.
    pub after: Option<i64>,
}

#[utoipa::path(get, path = "/events", params(EventsQuery),
    responses((status = 200, description = "text/event-stream: first an `event: cursor` frame carrying the start id, then `event: activity` frames whose `id` is the entry id and whose data is ActivityEntryDto JSON", content_type = "text/event-stream")))]
async fn events(
    State(s): State<AppState>,
    c: Caller,
    headers: HeaderMap,
    ApiQuery(q): ApiQuery<EventsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let resume = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .or(q.after);
    // A resume point above the newest committed entry (a stale client after a restore, or a forged id) would otherwise
    // make the stream silently miss every later write; clamp it. No resume point: start after the newest entry.
    // A database error here is an ordinary 500.
    let newest = s
        .activity
        .max_id()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    let start = resume.map_or(newest, |id| id.min(newest));
    // Re-checked every tick (kb/ADR-0008 R2): a revoked, expired or disabled session ends the stream.
    let secret = c.secret().to_string();
    let poll = Duration::from_millis(s.config.events.poll_ms);
    let polls = futures_util::stream::unfold(
        (start, s, secret, true),
        move |(mut last, s, secret, catch_up)| async move {
            // Sleep between polls, except for the first and after a full batch: more rows are probably waiting.
            if !catch_up {
                tokio::time::sleep(poll).await;
            }
            // The browser's EventSource reconnects, gets 401 and goes to sign-in.
            if reeve_identity::authenticate(&*s.tokens, &s.identity, &secret, Utc::now())
                .await
                .is_err()
            {
                return None;
            }
            // Any read error ends the stream too; the client reconnects with Last-Event-ID and misses nothing.
            let rows = s
                .activity
                .list(ActivityFilter {
                    after_id: Some(last),
                    limit: Some(BATCH),
                    ..Default::default()
                })
                .await
                .ok()?;
            let full = rows.len() as i64 >= BATCH;
            let mut events: Vec<Result<Event, Infallible>> = Vec::with_capacity(rows.len());
            for r in rows {
                let dto = super::activity::to_dto(r);
                last = last.max(dto.id);
                events.push(Ok(Event::default()
                    .event("activity")
                    .id(dto.id.to_string())
                    .json_data(&dto)
                    .expect("ActivityEntryDto serialises")));
            }
            Some((futures_util::stream::iter(events), (last, s, secret, full)))
        },
    )
    .flatten();
    // The first frame is always the cursor, so a client that disconnects before any activity still reconnects with a
    // Last-Event-ID; the client also refetches its views on every (re)connect.
    let cursor = futures_util::stream::once(async move {
        Ok(Event::default()
            .event("cursor")
            .id(start.to_string())
            .data(start.to_string()))
    });
    let sse = Sse::new(cursor.chain(polls))
        .keep_alive(KeepAlive::new().interval(KEEP_ALIVE).text("keep-alive"));
    // Tell reverse proxies (nginx and friends) not to buffer the stream.
    Ok(([("x-accel-buffering", "no")], sse))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(events))
}
