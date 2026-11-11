---
id: kb/ADR-0023
title: "ADR-0023: Attachments are raster images, checked and stripped on upload and served only through the request's access check"
description: "We decided that attachments are PNG, JPEG or WebP identified by their bytes, stripped of metadata without re-encoding, capped at 10 MiB, served sandboxed from the server under every request's access check and hard-deleted on request, because they share the session cookie's origin and every member can see them."
tags: [adr, reeve, planning, attachments, security]
status: accepted
date: 2026-11-10
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: one-way
blast_radius: shared
sensitivity: [security, privacy]
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve-protocol/**", "reeve:crates/reeve-server/**", "reeve:web/**", "reeve:deploy/README.md"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0006, kb/ADR-0015, kb/ADR-0022]
related: [kb/ADR-0010, kb/ADR-0016, kb/ADR-0019]
implements: []
rfc:
---

# ADR-0023: Attachments are raster images, checked and stripped on upload and served only through the request's access check

## Status
Proposed 2026-11-10.
Accepted 2026-11-11 by Daniel Thyselius.

## Context
Attachments (kb/ADR-0022) are uploaded by members and shown inline in Markdown on the same origin as the app, where
the browser session is a cookie (kb/ADR-0015). A file the browser renders as SVG or HTML on that origin can run
script with the viewer's session. Every signed-in person, guests included, is a full member and sees every item
(kb/ADR-0015), and a revoked person must lose access on their next request (kb/ADR-0015 R5). Screenshots can show
customer data; photos carry GPS position and device names in EXIF and XMP metadata. axum refuses request bodies
over 2 MB by default, and an nginx proxy refuses over 1 MB by default. Two parts of this decision are one-way: bytes
stripped of metadata cannot be restored, and a hard-deleted attachment cannot be recovered; the type list and the
cap can change later.

## Facts relied on
- F1 The browser session is the `__Host-reeve_session` cookie, and same-origin image GETs send it with no client header — source: `crates/reeve-server/src/auth/mod.rs`.
- F2 Every authenticated request rechecks the owner's access with no cache — source: kb/ADR-0015 R5.
- F3 The app's CSP allows images from `'self'` and `data:` — source: `crates/reeve-server/src/assets.rs` (the CSP header).
- F4 Markdown renders through `marked` and DOMPurify, which keep `<img>` with a same-origin or http(s) URL — source: `web/src/lib/markdown.ts`.
- F5 Guests are full members, with no per-item visibility — source: kb/ADR-0015.

## Considered options
- **Any file type, served as a download**: covers logs and PDFs too, but inline display needs a type the browser renders, and a type allowlist is needed anyway to prevent stored XSS.
- **Raster images stored as uploaded**: simple and lossless, but GPS and device metadata in a dropped photo reach every member.
- **Re-encode every image**: always clean and uniform, but lossy or slow on large images, and an image decoder on the server is new attack surface.
- **Presigned or public URLs for images**: cacheable by a CDN, but a URL outlives a revocation (F2).
- **Soft delete with an admin purge**: recovers mistakes, but adds a second concept and a role check, and an erasure request needs the bytes gone.
- **Raster images by magic bytes, metadata stripped without decoding, served sandboxed under the per-request check, hard delete** (chosen).

## Decision
We decided that attachments are PNG, JPEG or WebP identified by their bytes, stripped of metadata without
re-encoding, capped at 10 MiB, served sandboxed from the server under every request's access check and
hard-deleted on request, because they share the session cookie's origin (F1) and every member can see them (F5).

- **R1** An upload MUST be accepted only when its leading bytes identify PNG, JPEG or WebP, and MUST be stored with the type its bytes identify, whatever type the request declared; SVG, HTML, GIF and every other type MUST be refused.
- **R2** An upload larger than 10 MiB (10,485,760 bytes) MUST be refused; the larger body limit MUST apply to the upload route only.
- **R3** EXIF and XMP metadata, and PNG text chunks, MUST be removed before storage, without decoding or re-encoding pixels.
- **R4** Attachments MUST be served by the server binary only, with at least the stored content type, `X-Content-Type-Options: nosniff`, `Content-Disposition: inline`, `Content-Security-Policy: default-src 'none'; sandbox` and `Cache-Control: private, no-cache`; a conditional request MUST be answered 304 only after the access check of R5 and R6 passes.
- **R5** Every attachment request MUST pass the normal authentication and access check (kb/ADR-0015 R5); no presigned, public or shared-cache URL MAY be issued.
- **R6** A claimed attachment MUST be viewable and deletable by any member; an unclaimed one only by its uploader; any other authenticated request for it MUST answer 404.
- **R7** Deleting a claimed attachment MUST remove its row and chunks in the transaction that writes its activity entry; no activity entry MAY copy an attachment's content, and an unclaimed attachment MUST NOT appear in the activity log.

## Consequences
- Screenshots render inline with no CSP change (F3, F4), and a malicious upload cannot run script on the app's origin.
- A revoked person cannot fetch any attachment, cached or not, on their next request.
- Video (MP4, WebM) and GIF need a later record or a replacement that extends R1 and R3, with Range serving and their metadata removal.
- Negative: logs, PDFs, GIFs and other useful files cannot be attached.
- Negative: deleted bytes stay in backups until those rotate out; `deploy/README.md` must say so.
- Negative: no attachment is restorable after delete; a mistake means uploading again.
- Negative: every view revalidates with the server (a 304 when unchanged) instead of reading the browser cache.

## Assumptions and invalidation triggers
- *Assumes every member may see every attachment.* Trigger: guests need restricted visibility of items or attachments ⇒ supersede together with kb/ADR-0015.
- *Assumes screenshots are what people attach.* Trigger: requests for logs, PDFs or other files recur ⇒ revisit R1.
- *Assumes no format needs decoding on the server.* Trigger: a wanted format or feature (thumbnails, HEIC) needs a server-side decoder ⇒ supersede.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0015, kb/ADR-0019, kb/ADR-0022.
