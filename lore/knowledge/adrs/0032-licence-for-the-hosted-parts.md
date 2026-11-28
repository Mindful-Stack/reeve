---
id: kb/ADR-0032
title: "ADR-0032: The server side of Reeve is licensed AGPL-3.0-only, and the CLI and wire types stay MIT"
description: "We decided to license Reeve's server, its domain crates and its web client under AGPL-3.0-only, keeping the `reeve` CLI and `reeve-protocol` under MIT, because anyone who offers a changed Reeve as a service must then publish the change, while the tool people and agents install stays free of copyleft."
tags: [adr, reeve, legal, licence, hosting, cli]
status: accepted
date: 2026-11-27
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: one-way
blast_radius: shared
sensitivity: [legal]
scope: ["reeve:LICENSE", "reeve:LICENSE-MIT", "reeve:Cargo.toml", "reeve:crates/*/Cargo.toml", "reeve:web/package.json", "reeve:README.md"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0016]
related: [kb/ADR-0006, kb/ADR-0030]
implements: []
rfc:
---

# ADR-0032: The server side of Reeve is licensed AGPL-3.0-only, and the CLI and wire types stay MIT

## Status
Proposed 2026-11-27.
Accepted 2026-11-27 by Daniel Thyselius.

## Context
Reeve has been public under the MIT licence, which lets anyone run a changed copy as a hosted service without sharing the change. Mindful Stack is considering offering hosted Reeve instances, and the GPL's share-alike rule does not reach that case either, because running software on a server is not distribution. The AGPL's §13 adds that rule for software users reach over a network.

Copyleft has a price. Some companies forbid AGPL software outright, even for internal, unmodified use, and the `reeve` CLI is the tool people and agents install on their own machines and call from their scripts (it is Reeve's primary interface). Nobody offers a CLI as a hosted service, so copyleft on it would cost adopters and protect nothing.

The CLI links only `reeve-protocol` at run time; `reeve-server`, `reeve-identity` and `reeve-activity` are dev-dependencies for its tests. The server and the domain crates form one program, which the web client is embedded in.

Releases already published under MIT stay under MIT; a licence change applies from the commit that makes it.

## Facts relied on
- F1 The repository is licensed MIT throughout — source: `LICENSE`; `license = "MIT"` in `Cargo.toml`, inherited by every crate.
- F2 The `reeve` CLI's only internal run-time dependency is `reeve-protocol` — source: `[dependencies]` and `[dev-dependencies]` in `crates/reeve/Cargo.toml`.
- F3 Every commit on `main` is by Daniel Thyselius for Mindful Stack, or by Dependabot — source: `git log origin/main --format='%an'`.
- F4 AGPL-3.0 §13 requires anyone who modifies the program and lets users interact with it remotely to offer those users its corresponding source — source: https://www.gnu.org/licenses/agpl-3.0.html

## Considered options
- **Keep MIT throughout** — simplest, and no adopter is put off; a competitor could host a changed Reeve and keep the change.
- **AGPL-3.0-only for the whole repository** — one licence to explain; the CLI would carry copyleft that protects nothing and that some companies refuse.
- **AGPL-3.0-only for the server side, MIT for the CLI and `reeve-protocol`** — copyleft where hosting happens, none on what people install; two licences in one repository to keep straight.
- **A source-available licence (FSL, BSL)** — the strongest protection against a hosted competitor; it is not open source, which costs the trust of the developer and agent audience Reeve is for.
- **AGPL-3.0-or-later instead of -only** — follows future FSF versions automatically; hands the terms of later versions to a third party, while a sole copyright holder can relicense when it wants to.

## Decision
We decided to license `reeve-server`, `reeve-planning`, `reeve-identity`, `reeve-activity`, `reeve-testdb` and the web client under AGPL-3.0-only, and to keep the `reeve` CLI and `reeve-protocol` under MIT, because someone who offers a changed Reeve as a service must then publish the change, while the tool people and agents install stays free of copyleft.

- **R1** Every crate's `license` field and `web/package.json` MUST name its licence as an SPDX identifier: `AGPL-3.0-only` for the server side, `MIT` for `reeve` and `reeve-protocol`.
- **R2** The repository root MUST hold the AGPL text in `LICENSE` and the MIT text in `LICENSE-MIT`, and the README MUST say which part is under which.
- **R3** The `reeve` CLI and `reeve-protocol` MUST NOT take a run-time dependency on an AGPL crate; a dev-dependency is allowed.
- **R4** A contribution from anyone other than Mindful Stack MUST NOT be merged until its author has agreed to terms that let Mindful Stack relicense it.

## Consequences
- A company that hosts a changed Reeve must publish its changes; running an unchanged Reeve, or one changed only for internal use, costs nothing.
- Some teams whose policy forbids the AGPL will not self-host the server, even though their internal use would comply.
- Two licences in one repository: a reader must check which applies, and R3 needs watching when the CLI gains a dependency.
- R4 adds a step before the first outside contribution, and may put some contributors off.
- As the sole copyright holder, Mindful Stack can still sell a commercial licence or relicense later.

## Assumptions and invalidation triggers
- *Assumes Mindful Stack holds the copyright to all of the code.* Trigger: a contribution merged without relicensing terms, or a claim that another party owns the code ⇒ supersede.
- *Assumes the CLI need not link server code.* Trigger: the CLI needs an AGPL crate at run time ⇒ supersede.
- *Assumes copyleft is enough protection for a hosted offering.* Trigger: a competitor hosts Reeve in a way the AGPL does not prevent and it harms the offering ⇒ supersede.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0016 (one team per instance), kb/ADR-0006 (server and client stack), kb/ADR-0030 (CLI distribution).
