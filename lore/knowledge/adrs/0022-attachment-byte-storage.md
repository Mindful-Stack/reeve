---
id: kb/ADR-0022
title: "ADR-0022: Attachment bytes are stored in PostgreSQL as 1 MiB bytea chunks owned by the planning context"
description: "We decided to store attachment bytes in planning-owned PostgreSQL tables as 1 MiB bytea chunks, because the planner has one storage engine and one database container, and chunks let a read stream a file, and later serve a byte range, without holding the whole file in memory."
tags: [adr, reeve, planning, storage, attachments]
status: proposed
date: 2026-11-10
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve-server/**", "reeve:scripts/backup.sh", "reeve:scripts/restore-rehearsal.sh"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0007, kb/ADR-0010, kb/ADR-0016]
related: [kb/ADR-0002, kb/ADR-0023]
implements: []
rfc:
---

# ADR-0022: Attachment bytes are stored in PostgreSQL as 1 MiB bytea chunks owned by the planning context

## Status
Proposed 2026-11-10.

## Context
People want to paste screenshots into the descriptions, briefs and comments of initiatives, epics and stories, and
screen recordings are expected in a later slice. The planner stores all planning data in PostgreSQL through sqlx with
no second storage engine (kb/ADR-0007 R1), and the reference deployment is one server container and one PostgreSQL
container (kb/ADR-0016 R3). Backups are a `pg_dump --format=custom` archive, checked by a restore rehearsal. This
slice caps a file at 10 MiB, where one `bytea` value per file would serve; screen recordings can reach about 100 MB,
sqlx reads a `bytea` value whole, and browsers play video through HTTP Range requests. The chunk layout is chosen now
so the video slice needs no data migration. Items and comments live in the planning context (kb/ADR-0010 R3), so
tables there may hold foreign keys to them. The planner has no background job runner.

## Facts relied on
- F1 No second storage engine may be introduced for planning data — source: kb/ADR-0007 R1.
- F2 The reference deployment is one server and one PostgreSQL container — source: kb/ADR-0016 R3; `docker-compose.yml`.
- F3 The restore source is a `pg_dump` custom-format archive — source: `deploy/README.md` "Backup and restore"; `scripts/backup.sh`.
- F4 sqlx decodes a `bytea` column into one in-memory buffer — source: sqlx `Decode` for `Vec<u8>`.
- F5 (forward-looking, for the video slice) Safari plays MP4 only from a server that answers Range requests — source: Apple's Safari HTML5 media documentation.

## Considered options
- **One `bytea` value per file**: the simplest schema and code, and enough for 10 MiB screenshots, but every read holds the whole file in memory, and video would need a data migration to chunks.
- **PostgreSQL large objects** (`lo_*`): streaming and seeking are built in, but they have their own ownership and dump semantics, `pg_restore --no-owner` and orphan cleanup need extra care, and sqlx has no direct support.
- **A filesystem volume on the server container**: streaming and Range are trivial, but the server becomes stateful and backups need a second artefact kept consistent with the dump; it is a second storage engine (F1).
- **S3-compatible object storage** (MinIO alongside, or a hosted bucket): built for blobs at any scale, but it adds a container or an external service (F2), its own credentials and its own backup, and presigned URLs would bypass the per-request access check.
- **1 MiB `bytea` chunks in planning-owned tables** (chosen): one engine and one dump, bounded memory per read, Range reads later touching only the chunks they need, and foreign keys to items.

## Decision
We decided to store attachment bytes in planning-owned PostgreSQL tables as 1 MiB `bytea` chunks, because the
planner has one storage engine and one database container (F1, F2), and chunks let a read stream a file, and later
serve a byte range, without holding the whole file in memory (F4, F5).

- **R1** Attachment bytes MUST be stored only in PostgreSQL, in `attachment_chunks` rows of at most 1 MiB keyed by attachment and chunk number; no filesystem, object store or large object MAY hold them.
- **R2** Attachment metadata and chunks MUST be owned by the planning context; an attachment MUST be owned by at most one initiative, epic or story, and one owned by none is an upload not yet claimed.
- **R3** Serving an attachment MUST read its chunks one query at a time, in order, and MUST NOT hold the whole file in memory; an upload MAY hold its body in memory up to the upload cap.
- **R4** Every upload MUST delete, in its own transaction, every unclaimed attachment older than seven days; the planner MUST NOT run a background job for it.
- **R5** The JSON export MUST carry attachment metadata only; the bytes MUST travel in the PostgreSQL dump.

## Consequences
- The existing backup, restore and restore rehearsal cover attachments with no new artefact.
- Range serving for video needs no storage change.
- Deleting an attachment is a row delete that cascades to its chunks.
- Negative: the dump, and the time to take and restore it, grow with every attachment; a few gigabytes of screenshots make backups visibly slower.
- Negative: chunking is code to write and test that a single value would not need in this slice.
- Negative: PostgreSQL TOAST compression gains nothing on already-compressed images, so storage equals the bytes uploaded.
- Negative: unclaimed uploads persist until the next upload after they turn seven days old; on a quiet instance that can be much longer.

## Assumptions and invalidation triggers
- *Assumes attachments stay in the low gigabytes.* Trigger: attachment chunks exceed 20 GB, or a nightly dump takes longer than 15 minutes ⇒ supersede with an object store and replace kb/ADR-0007 R1 for attachments.
- *Assumes one PostgreSQL host serves the instance.* Trigger: a move to a managed database with storage limits or per-GB cost that makes blobs in the database expensive ⇒ supersede.
- *Assumes no job runner exists.* Trigger: the planner gains a scheduled job runner ⇒ revisit R4.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0007, kb/ADR-0016, kb/ADR-0023.
