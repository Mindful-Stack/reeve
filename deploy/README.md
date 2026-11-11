# Deploying the Reeve planner server

This is the reference deployment: one Docker Compose stack (`docker-compose.yml`) with PostgreSQL and the server, built
from this repository or run from the image every push to `main` publishes (`ghcr.io/mindful-stack/reeve:<sha>`). An
operator's own production setup (hosts, users, secrets, a deploy job) lives outside this repository.

1. Copy `reeve.example.toml` to `reeve.toml` in the repository root (next to `docker-compose.yml`; compose mounts `./reeve.toml`, and Docker creates a directory of that name if the file is missing).
2. Set the database and bind address. The same file name needs different values depending on how the server runs, so one `reeve.toml` cannot serve both:
   - `mise run run` (server on the host, Postgres published on 127.0.0.1:5433):
     `database_url = "postgres://reeve:reeve@127.0.0.1:5433/reeve"`
   - `docker compose up` (server inside the compose network):
     `database_url = "postgres://reeve:reeve@postgres:5432/reeve"` and `bind = "0.0.0.0:7474"`

   Without an `[oidc]` section `public_url` may be left out: it defaults to `http://<bind>`, or to `http://127.0.0.1:<port>` for a `0.0.0.0` or `::` bind. Browser sign-in needs it set (next section).
3. Start the stack: `docker compose up -d`. The API is published on `127.0.0.1:7474` only; put a TLS reverse proxy in front of it for anything beyond localhost.
4. Issue the first token. `--email` must be allowed by the config (`allowed_domains` or `allowed_emails`; `reeve.example.toml` allows only `example.com`):
   `docker compose exec server reeve-server token-issue --email you@your-domain --kind interactive`
5. On a developer machine, keep the token out of the process list:
   `echo "<token>" | reeve auth set-token --url http://localhost:7474 --token-stdin`

The container image carries the web client (built in its own Node stage). A server built on the host embeds whatever `web/build` holds when it is compiled: run `mise run web` (Node 22 or later) before `cargo build`, or every page answers `503 client not built`.

## Subscribe to releases

If you deploy the published image from a repository of your own (a pin bumped by a reviewed pull request, say), you
can have every release start a workflow there within minutes of the image being published. Then nothing has to poll
on a schedule, which GitHub may start hours late.

**To subscribe:**

1. **Install the [reeve-releases](https://github.com/apps/reeve-releases) GitHub App** on your deploy repository,
   choosing *Only select repositories* and selecting just that one. The App asks for one permission, **Actions: read
   and write**, so all it can do there is start, re-run and cancel workflows. It can't read your code or secrets, push,
   or open pull requests. Uninstalling it unsubscribes you.
2. **Add `.github/workflows/reeve-release.yml`** on the repository's default branch. It must accept `workflow_dispatch`
   with two string inputs, `commit` and `digest`. GitHub refuses a dispatch whose inputs the workflow doesn't declare.

```yaml
name: reeve-release
on:
  workflow_dispatch:
    inputs:
      commit: { description: "The Reeve commit just published (40 hex characters)", required: false, default: "" }
      digest: { description: "Its image digest (sha256:…)", required: false, default: "" }
permissions: { contents: read }
jobs:
  bump:
    runs-on: ubuntu-24.04
    steps:
      # Treat the inputs as claims: pass them through env (never ${{ inputs.* }} inside run:) and check them against
      # the registry before acting on them.
      - name: the release is real
        env: { COMMIT: "${{ inputs.commit }}", DIGEST: "${{ inputs.digest }}" }
        run: |
          [[ $COMMIT =~ ^[0-9a-f]{40}$ && $DIGEST =~ ^sha256:[0-9a-f]{64}$ ]] || { echo "::error::malformed inputs"; exit 1; }
          got=$(docker buildx imagetools inspect "ghcr.io/mindful-stack/reeve:$COMMIT" --format '{{.Manifest.Digest}}')
          [ "$got" = "$DIGEST" ] || { echo "::error::the published image for $COMMIT is $got, not $DIGEST"; exit 1; }
      # ...then propose your own change: bump your pin to ghcr.io/mindful-stack/reeve@$DIGEST in a pull request.
```

The release workflow's `notify` job runs after every successful publish on `main`. It starts `reeve-release.yml` on
the default branch of every repository where the App is installed. A repository without the file is skipped, and so is
one that refuses the dispatch. This repository's run logs are public, so they show only counts and HTTP statuses, never
a repository's name; check delivery in your own repository's Actions tab. The dispatch is best effort: a release
that fails to reach you is not retried, so keep a daily check of your own as a fallback.

**Running the App (maintainers and forks).** The `notify` job reads two repository secrets: `RELEASE_APP_ID` (the
App's ID or client ID) and `RELEASE_APP_PRIVATE_KEY` (a private key generated in the App's settings, the whole PEM
file). With neither set it does nothing, which is how a fork starts. For a fork with subscribers of its own, create a
GitHub App with no webhook, the single repository permission *Actions: read and write*, and *Any account* allowed to
install it, then set the two secrets. The job makes a JSON Web Token from the key, asks GitHub for a one-hour,
Actions-only token per installation, and dispatches to at most 100 repositories per release (`RELEASE_DISPATCH_MAX`),
so a stranger installing the App on many repositories can't stall a release. Rotate the key in the App's settings:
generate a new one, update the secret, delete the old one. Subscribers are not involved.

## Google sign-in

Browser sign-in uses Google as the OpenID Connect provider. Without an `[oidc]` section the web client cannot sign anyone in, and only tokens issued with `reeve-server token-issue` work.

1. In the Google Cloud Console (APIs & Services → Credentials), create an OAuth 2.0 client ID of type **Web application**. Add the authorised redirect URI `https://<host>/auth/callback`, where `<host>` is the address people use to reach the planner.
2. Put the client's credentials in `reeve.toml`:

       [oidc]
       provider = "google"
       client_id = "1234-abc.apps.googleusercontent.com"
       client_secret = "..."

3. Set `public_url = "https://<host>"`. It is required whenever `[oidc]` is configured; the server refuses to start without it. It has no path; the redirect URI is `<public_url>/auth/callback`, so the two must match exactly. Optionally set `session_days` (how long a browser session lasts, 1 to 365; default 30). Who may sign in is still decided by `allowed_domains` / `allowed_emails`, and `admins` decides who sees the users page.
4. Restart the server. An address listed in `admins` signs in at `https://<host>/`. From then on, people issue their own tokens on the **Tokens** page, and `reeve auth login --url https://<host>` opens that page and stores the interactive token pasted back into the terminal.

### Reverse proxy

The proxy must forward `/auth/*`, `/api/*`, `/webhooks/*` (the GitHub webhook, when configured) and `/` (the web client, every other path) to the server, over HTTPS. Session cookies are `Secure` whenever `public_url` is https, so a browser never sends them over plain HTTP: terminate TLS at the proxy and keep `public_url` on https.

The web client keeps one server-sent-events stream open per tab (`/api/v1/events`). The proxy must not buffer it, and its read timeout must be above 15 s (the server sends a keep-alive every 15 s). Serve HTTP/2 to browsers: over HTTP/1.1 a browser allows only six connections per host, and every open tab holds one. For nginx:

    location /api/v1/events {
        proxy_pass http://127.0.0.1:7474;
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        proxy_buffering off;
        proxy_read_timeout 1h;
    }

    location / {
        proxy_pass http://127.0.0.1:7474;
    }

with `listen 443 ssl; http2 on;` in the server block. Caddy's `reverse_proxy` streams server-sent events without buffering by default (no `flush_interval -1` needed) and serves HTTP/2 over TLS out of the box:

    planner.example.com {
        reverse_proxy 127.0.0.1:7474
    }

## Access

`allowed_domains`, `allowed_emails` and `admins` in `reeve.toml` are the locked floor: nobody can remove what they name from the web client. Admins add guests, domains and further admins on the **Access** page (in the account menu); those live in the database (`allowed_entries` and `admin_grants`) and the page lists them beside the floor. The server refuses to start if `admins` names no one, or names an admin that the floor's own `allowed_domains` or `allowed_emails` do not allow; blank entries are ignored. The policy is read from the database on every request, so an Access change takes effect at once, with no restart.

`reeve-server --config <reeve.toml> access-list` prints the merged lists, one line per entry with four tab-separated columns: kind (`domain`, `email` or `admin`), value, source (`reeve.toml`, or who added it and when) and `not allowed` when the entry is inert. It is read-only: it never migrates, and it refuses a database at another migration.

**Restore.** The access tables are in the dump; the floor comes from the `reeve.toml` the restored server runs with. A grant the restored floor no longer allows is kept, marked `not allowed` and inert (it grants nothing), until an admin revokes it on the Access page or the floor allows it again.

**Admin recovery.** When every admin is unusable (disabled, or no longer able to sign in), add a fresh Google identity to both `allowed_emails` and `admins` in `reeve.toml` and restart the server. That person signs in as admin and re-enables the others. A configured admin can be disabled only on the host, with `reeve-server user-disable`.

## Rate-limit browser sign-in at the reverse proxy

`/auth/login` is unauthenticated, and each call holds a pending sign-in in memory for ten minutes. The server keeps at most 1 000 of them and, when full, refuses new sign-ins with a 503 page ("too many sign-ins in progress, try again in a minute") rather than evicting one already in progress. Rate-limit `/auth/login` per client IP at the proxy so one client cannot fill it; for nginx, 10 requests a minute per IP:

    # in the http block
    limit_req_zone $binary_remote_addr zone=reeve_login:10m rate=10r/m;

    # in the server block, before the general proxy location
    location = /auth/login {
        limit_req zone=reeve_login burst=5 nodelay;
        proxy_pass http://127.0.0.1:7474;
    }

## GitHub webhook

Pull requests whose branch names a story (`ABC-12-fix-login`, `feature/abc-12`; the key starts the branch's last path segment, any case) show on that story with their status: draft, open, merged or closed. The status is informational only: a merged pull request never moves a story. Branches named after a Jira key from before the import (`SCRUM-11-old`) resolve to the story imported from that key when `SCRUM` is listed in `legacy_prefixes`: list the Jira delivery project key (stories, bugs, tasks), not the discovery project, whose ideas import as initiatives.

1. Add a `[github]` section to the configuration and restart the server:

        [github]
        webhook_secret = "<a random string of at least 16 characters, e.g. openssl rand -hex 32>"
        repos = ["Acme/portal"]   # owner/name of every repository whose events are accepted
        legacy_prefixes = ["SCRUM"]   # optional: the Jira delivery project key; its branch names resolve through external_ref

   Without the section `/webhooks/github` answers 404.
2. In each repository listed in `repos`: **Settings → Webhooks → Add webhook**. Payload URL `https://<host>/webhooks/github`, content type `application/json`, the same secret, and "Let me select individual events" with only **Pull requests** ticked. GitHub sends a `ping` on creation; a green tick on it means the signature checked out.
3. The reverse proxy forwards `/webhooks/*` to the server (see above).

Every delivery must carry a valid `X-Hub-Signature-256` for the secret; anything else gets 401. Events from repositories not in `repos`, branches that name no story, pull requests from forks (a head repository other than the repository itself, or none: a deleted fork), events other than pull requests, and events that change nothing visible (a push, a label) get 202 and change nothing. Bodies over 2 MB are refused with 413 before the handler runs; that is far above pull-request payload sizes (tens of KB). The activity log records each link and status change as actor `github` via `system`.

### Failed deliveries

GitHub does not retry a delivery that got a non-2xx answer. The webhook answers 500 only when the database failed (logged with the `X-GitHub-Delivery` id), so after a database outage open the repository's **Settings → Webhooks →** the Reeve hook **→ Recent Deliveries**, pick each delivery with a non-2xx response and press **Redeliver**. With the GitHub CLI:

    gh api repos/<owner>/<repo>/hooks                                   # find the hook id
    gh api repos/<owner>/<repo>/hooks/<hook_id>/deliveries              # list deliveries with their status codes
    gh api -X POST repos/<owner>/<repo>/hooks/<hook_id>/deliveries/<delivery_id>/attempts   # redeliver one

A redelivery is safe: an event older than what is stored is ignored, and a repeat of the stored state changes nothing. One edge: redelivering the earlier of two events that carry the same `updated_at` flips the status back and logs `pr_updated` again; the next event on that pull request corrects it.

## Board settings

The board asks for stories with `GET /api/v1/stories?done=recent`, which leaves out stories done longer ago than a window; the other columns are unaffected, and the done column's heading says how far back it reaches ("done · last 14 days"). The window is configured in `reeve.toml`:

    [board]
    done_window_days = 14

- `done_window_days` (default `14`, allowed `1` to `3650`; a value outside the range stops the server at start) is how many days back a done story still counts as recent. The cutoff is computed from the request time. Stories imported already done have no completion time and are never recent. `?done=all` (the default for API callers) returns every story.

`[events] poll_ms` (default 2000, 10–10000): how often the live event stream reads the activity log. Lower means
fresher boards and TUIs at the cost of one small query per open stream per interval.

## Cut-over day: importing Jira

`reeve import jira` reads the Jira Product Discovery project (ideas → initiatives) and the software project (epics, stories, bugs, tasks and their comments) through the Jira Cloud API and creates them in Reeve. Without `--apply` it is a rehearsal: it reads everything, prints the report and writes nothing. The importer only creates: every item keeps its Jira key as its external reference (`SCRUM-11`, comments as `jira-comment:<id>`), so a second `--apply` against the same database creates nothing and reports everything as existing. Stories in progress, in review or testing are created in that state with the label `legacy-import`, and the epic and initiative above each get a discovery mark by the importing admin; none of this is an override, so the override count does not see it.

What is not imported: Confluence, sprints, attachments beyond a link, and comments on ideas and epics (those are never read). Comments are read and imported for stories, bugs and tasks; the comments of an issue that is skipped are listed as skipped.

### The mapping file

Copy `jira-import.example.toml` to `jira-import.toml` and adjust it to the site; `--mapping jira-import.toml` names it on every run.

- `site` is the Jira site (`--site <url>` overrides it); `api_base` is needed only for scoped API tokens (`https://api.atlassian.com/ex/jira/<cloudId>`). Both must be https; plain http is accepted only for a loopback host (a local fake or proxy). `email` plus the API token in `JIRA_TOKEN` (or the variable named by `--token-from-env`) is the basic auth.
- `discovery_project` and `delivery_project` are the two project keys: each an uppercase letter followed by uppercase letters or digits, and the two must differ. `bucket_field` and `target_date_field` are the site's custom field ids for the idea bucket and target date.
- `idea_link_type` is the link type joining ideas to delivery epics. It has no default, and `--apply` refuses an empty one; a rehearsal lists every link type it saw, so take the name from there.
- `[bucket_to_horizon]` values must be horizon buckets of the instance (`horizon_buckets` in `reeve.toml`); `[status_to_delivery]` maps to `todo`, `in_progress`, `in_review`, `testing`, `done`; `[resolution_map]` and `default_resolution` to `completed` or `dropped`; `[type_to_kind]` to `story`, `bug` or `chore`. An idea with an unmapped bucket, an issue with an unmapped status or type, is skipped and reported.
- `[owner_map]` maps a Jira accountId to an email for assignees whose email Jira hides. An assignee resolved neither way is reported and the owner is left empty. Owners are stored as text; the import never creates users.
- `acceptance_heading` (default `Acceptance criteria`) is the description heading whose section becomes a story's criteria.
- Epics are the issues whose type is named exactly `Epic`. A site that renamed or localised its Epic type sees every epic skipped as an unmapped type: rename the type back in Jira for the import, or extend the importer.

The import needs an admin (`admins` in `reeve.toml`) with an interactive token; `--apply` additionally needs `[import] enabled = true` on the server (without it the import endpoints answer 404, and the CLI stops before reading Jira with "import is switched off on this instance"). Every run, a rehearsal included, checks the mapping against the instance (allowed values, `horizon_buckets`, project keys, URLs) before reading Jira and stops on the first error. Exit status: 0 success; 2 the server refused the caller (not an admin with an interactive token, or a create answered `forbidden` or `unauthenticated`); 1 any other failure (a mapping error, a missing Jira token, the switch off, a Jira page that failed, a create that failed validation or got no answer). With the global `--json` flag every report prints as `{"outcome":"ok","value":<report>}`, partial failures included (they are in `value.apply.failed`, and the exit status says so); a run that ends without a report prints the error envelope with code `forbidden`, `validation` (mapping, token, switch off) or `internal` (a Jira page that failed, with `page`, `status` and `written: false`).

### The day

1. **Rehearse against the real site, days ahead.** On an instance configured like the target (the same `horizon_buckets`), as an admin with an interactive token:

        JIRA_TOKEN=<api token> reeve import jira --mapping jira-import.toml

   Show the report to the team. It lists the planned initiatives, epics, stories and comments (by kind and by delivery state), the link types seen, the stories that would be grandfathered and their epics and initiatives, and the warnings in groups: skipped ideas, orphan epics (no idea link), orphan epics (idea skipped), skipped issues, skipped comments, stories whose parent is not in the plan, the default resolution applied, owners left empty and titles truncated to 200 characters. Fix the mapping until the report is right: buckets and statuses until nothing is skipped by mistake, `idea_link_type` from the link types the report lists, and an `owner_map` entry for every unresolved owner. Check the two orphan groups by hand: an epic with no idea link imports without an initiative, and so does an epic whose idea was skipped. A rehearsal that fails to read a page ends with "nothing written: a partial fetch is never imported"; nothing is ever imported from a partial fetch.
2. **Announce the cut-over and set Jira read-only.**
3. **Import into a fresh database.** Stop the server and start from an empty database: with the reference compose file, `docker compose down -v` (it removes the stack's `reeve-pg` volume, so the rehearsal's data goes with it). Set `[import] enabled = true` in `reeve.toml` (the admin must be listed in `admins`), start the stack (`docker compose up -d`) and run `docker compose exec server reeve-server migrate` (`serve` migrates on start as well; the explicit run reports a failure directly). Issue the admin's interactive token (`docker compose exec server reeve-server token-issue --email <admin> --kind interactive`, then `echo "<token>" | reeve auth set-token --url https://<host> --token-stdin`). Without compose, do the same by hand: stop the server, drop and recreate its PostgreSQL database (or point `database_url` at a new, empty one), edit the server's `reeve.toml`, start the server, and run `reeve-server migrate` and `reeve-server token-issue` on the host with the same config (`--config` or `REEVE_CONFIG`). Run the import and keep its report (`pipefail` keeps the import's exit status through `tee`):

        set -o pipefail
        JIRA_TOKEN=<api token> reeve import jira --mapping jira-import.toml --apply | tee import-report-<date>.txt

   The apply creates initiatives, then epics, then stories, then comments. A create that fails is listed at the end with its Jira key, step, code and message, and everything below it is reported "skipped: parent <key> failed" and never sent; the rest continues. A failure that refused the caller (`forbidden`, `unauthenticated`) or got no answer stops the run: nothing more is sent, and every remaining item is reported "skipped: run stopped after <key> failed (<code>: <message>)". After fixing the cause, running `--apply` again is safe: what exists is reported as existing and only the missing items are created.
4. **Take the first export:** `reeve export --out backup-<date>.json`. It is the whole instance as one JSON document, written with mode 0600 (it holds every user's email): the readable record. The restore source is a database dump (see "Backup and restore"); take the first backup now and rehearse the restore before the team starts writing.
5. **Switch the import off:** set `[import] enabled = false` (or remove the section) and restart the server (`docker compose restart server`), so nobody can later create in-progress stories or back-dated comments through the import endpoints.
6. **Configure the GitHub webhook** (see "GitHub webhook" above) and verify it with a test pull request whose branch names a story: it shows on the story, and the activity log records `pr_linked` by `github`. Deliveries that failed are not retried by GitHub; redeliver them by hand (see "Failed deliveries").
7. **Confirm the scheduled decision check** (see "Scheduled decision check"): `systemctl list-timers reeve-decision-check.timer` shows the next run, `systemctl --failed` does not list it, and on the decisions page (reloaded) a decision with a record says "checked" within the last 15 minutes.
8. **Rollback, same day only.** Until anyone has written to Reeve, rolling back is re-enabling Jira writes and telling the team. From the next day, fix forward in Reeve.

Things to know:

- A re-run after a change to the mapping or to `reeve.toml` (a horizon bucket renamed or removed in `reeve.toml`, a different key prefix) may answer a validation error for an item that already exists, instead of reporting it as existing: every item is validated before it is looked up. The fix is to run with the mapping and configuration of the first apply.
- An external reference must look like a Jira key (`SCRUM-11`) and must not start with the instance's own prefix (`ABC-`): import into an instance whose `key_prefix` differs from both Jira project keys. A comment needs a non-blank body and author (the importer writes `_(empty comment)_` for an empty Jira comment, and an author Jira does not name as `Unknown (Jira)`), and an item needs a non-blank title: an issue with an empty summary fails its create with a validation error, and its comments are skipped.
- The export's `exported_at` is the time the request was authenticated, taken before the snapshot's first statement, so a change committed in between is in the document even though it is later than `exported_at`.

## Scheduled decision check

The planner never reads a record itself (kb/ADR-0004 R1): `reeve decision check` reads each linked record on its repository's default branch from a *household* of clones and posts what it found. In the reference stack it runs every ten minutes inside the server container, started by a host timer; the household lives in the `reeve-household` volume, which the server process never reads, and needs no backup (the next run clones whatever is missing).

Only an admin's check stores *accepted*, the result that satisfies a decision. A member's `reeve decision link` still runs its immediate check; when that finds the record accepted it says `held: accepted on the default branch; stored by the next scheduled check run by an admin`, exits 0, and the decision stays "unchecked" until the scheduled run. Every other result is stored from any caller.

A run logs a decision's result in the activity log (`checked`) only when it changed; a run that finds what the last one found moves only the decision's "checked" time, so the activity page keeps people's entries rather than one per decision per run. A decision someone links, relinks, edits (title, decider or due date) or closes while a run is under way is *skipped* by that run (the journal says `skipped: it changed … during the run, so the next run checks it`), not a failure: the next run checks its new state. Such a run sends the page no live update, so an open decisions list (on the decisions page, an epic, an initiative or a story) reloads itself every two minutes while the tab is visible, and the "checked" time stays current without a reload.

### Upgrading: repository names

From this release a `[[repos]]` name must be a single directory name: not empty, not starting with `.` or `-`, and without `/` or `\`. The rule is checked when the configuration loads, so an instance whose `reeve.toml` has, say, `name = ".github"` stops starting (`serve` and every other subcommand exit 1 with `[[repos]] name ".github" must be a single directory name: …`). Before upgrading, check the names (`grep -A1 '^\[\[repos\]\]' reeve.toml`). To rename one:

1. Note the decisions that link the old name: the `record.repo` of each in `reeve --json decision ls`.
2. Change `name` in `reeve.toml` (for example `.github` to `github-org`; the name is the planner's own label, the `url` still names the repository) and start the new version.
3. Relink each of those decisions under the new name: `reeve decision link <id> --repo <new name> --path <same path> --record <same record id> --household <household>` (add `--pr` if it had one). Until the relink the stored reference names a repository that is no longer configured, so the check reports it unreadable and the scheduled run fails. A relink resets the decision's check result to unchecked, which blocks the stories it guards, and checks it at once from `--household`: a directory that holds a clone of the repository at `<household>/<new name>`. The scheduled run's household is `/var/lib/reeve/household` in the server container, and its clone is still under the old name, so on a machine without such a clone (and inside the container, until the next run clones the new name) the immediate check reports the record unreadable and the command exits 1. The link is stored all the same, and the next scheduled run (within ten minutes) clones the repository under its new name and stores the result.

### Set it up

1. **Repositories.** Give every `[[repos]]` entry in `reeve.toml` the `url` to clone from, without credentials in it. `docker compose exec server reeve-server repos` prints what the check will clone (`name<TAB>url`) and warns about an entry without a url. Public repositories need nothing more; private ones need a deploy key (next section).
2. **A token for the check.** The check posts as an admin with an automation token. Give it a dedicated identity rather than a person's, so disabling a person does not stop the check: add an address such as `reeve-check@your-domain` to `allowed_emails` and `admins` (it never signs in, so it needs no Google account), run `docker compose restart server`, then issue its token straight into a root-only file, so it is never shown:

        sudo install -d -m 0700 /etc/reeve
        cd /opt/reeve && sudo sh -c 'umask 077; docker compose exec -T server reeve-server token-issue --email reeve-check@your-domain --kind automation --label decision-check > /etc/reeve/decision-check.token'

   `/opt/reeve` stands for the directory holding `docker-compose.yml` throughout this section. The token can do what an admin can, including disabling users: keep the file root-only, and if it leaks run `docker compose exec server reeve-server user-disable --email reeve-check@your-domain` (it revokes the token; issue a new one after re-enabling the identity from the web client's users page).
3. **Check that compose passes standard input and the exit status** (the timer relies on both):

        cd /opt/reeve && sudo docker compose exec -T server reeve-decision-check </dev/null; echo "exit $?"

   It must print `reeve-decision-check: no token: set REEVE_TOKEN or pass an admin's automation token on standard input` and `exit 2`. `exit 0` means this compose does not pass the exit status through, and the timer could not report a failed run.
4. **Install the timer.** The service runs `docker compose exec -T server reeve-decision-check` in `WorkingDirectory=/opt/reeve` with the token file as its standard input; change `WorkingDirectory=` if the stack lives elsewhere (it must be the directory `docker compose up` runs from, so compose finds the same project).

        sudo cp deploy/systemd/reeve-decision-check.service deploy/systemd/reeve-decision-check.timer /etc/systemd/system/
        sudo systemctl daemon-reload
        sudo systemctl start reeve-decision-check.service
        systemctl status reeve-decision-check.service
        sudo systemctl enable --now reeve-decision-check.timer
        systemctl list-timers reeve-decision-check.timer

   The first run clones every repository (`reeve-decision-check: cloning <name>` in the journal) and ends with the check's summary (`checked N decision(s), 0 held, 0 skipped, 0 unreadable, 0 failed`); `status` then shows `status=0/SUCCESS`.
5. **Look at the decisions page** (reload it): every decision with a record says "checked" within the last 15 minutes. The timer fires every ten minutes, each start up to two minutes late (`RandomizedDelaySec=60` and systemd's one-minute accuracy), the run itself takes a while, the open page reloads the list every two minutes and its label moves once a minute, so a healthy decision can read "checked 14m ago"; one older than 15 minutes means runs are failing or not starting. The timer has no `Persistent=`: after the host boots, the first run is the next ten-minute tick (a catch-up run at boot would start before compose has the server up and fail on every boot). That tick can still fall right after boot, before the server is up: that first run may fail once (a red unit in `systemctl status`), and the next tick recovers.

Without systemd, a cron entry (`/etc/cron.d/reeve-decision-check`) does the same; `flock` keeps runs from overlapping, and a failure is visible only in the log:

    */10 * * * * root cd /opt/reeve && flock -n /run/lock/reeve-decision-check docker compose exec -T server reeve-decision-check < /etc/reeve/decision-check.token >> /var/log/reeve-decision-check.log 2>&1

### Private repositories

A GitHub deploy key opens one repository, so each private repository gets its own read-only key and an ssh host alias that selects it. On the host:

    sudo install -d -m 0700 /etc/reeve/ssh
    sudo ssh-keygen -t ed25519 -N '' -C reeve-check-knowledge-base -f /etc/reeve/ssh/knowledge-base
    ssh-keyscan github.com | sudo tee /etc/reeve/ssh/known_hosts

Add `/etc/reeve/ssh/knowledge-base.pub` to the repository as a deploy key without write access, and compare the `known_hosts` lines with GitHub's published SSH key fingerprints. Then write `/etc/reeve/ssh/config` (paths are as the container sees them):

    Host github-knowledge-base
      HostName github.com
      User git
      IdentityFile /var/lib/reeve/ssh/knowledge-base
      IdentitiesOnly yes
    Host *
      BatchMode yes
      StrictHostKeyChecking yes
      UserKnownHostsFile /var/lib/reeve/ssh/known_hosts

and use the alias in the url: `url = "git@github-knowledge-base:Acme/knowledge-base.git"`. The web client's link to a record is built from this url too (`https://<host>/<org>/<repo>/blob/HEAD/<path>`, GitHub's layout; it works for `https://`, scp-style `git@host:org/repo` and `ssh://git@host/org/repo` urls), so the link carries the url's host: an ssh host alias like `github-knowledge-base` is not a web host, and the link then does not resolve. A host without a dot (an alias) gets no link at all, so a record in such a repository shows without one. Operators who use aliases should set the repository's web address in `reeve.toml` next to its `url`:

    [[repos]]
    name = "knowledge-base"
    url = "git@github-knowledge-base:Acme/knowledge-base.git"
    browse_url = "https://github.com/Acme/knowledge-base"
    adr_dir = "docs/adr"

`browse_url` must be an `https://` URL whose host has a dot (anything else stops the server starting); it takes precedence over the address derived from `url`, and records link to `<browse_url>/blob/HEAD/<path>`. Mount the directory and name the config in `docker-compose.override.yml` next to `docker-compose.yml` (compose merges it on `up`), then `docker compose up -d`:

    services:
      server:
        environment:
          GIT_SSH_COMMAND: ssh -F /var/lib/reeve/ssh/config
        volumes:
          - /etc/reeve/ssh:/var/lib/reeve/ssh:ro

The keys must stay root-only (`ssh-keygen` writes them 0600; ssh ignores a key others can read). Do not add `ControlMaster`/`ControlPersist` for these hosts: a persisting master outlives the git that started it, keeps the run's lock open and leaves the deadline's process group, so the next run finds the lock held and fails (exit 2). The script never changes an existing clone: to change a repository's url later, run `docker compose exec server git -C /var/lib/reeve/household/<name> remote set-url origin <url>`.

### When a run fails

The run's exit status is the unit's: `systemctl --failed` lists `reeve-decision-check.service`, and `journalctl -u reeve-decision-check.service` says why. 1: a record could not be read (a missing clone, a failed fetch, an unset `origin/HEAD`), a result was not stored, or a result was held because the token is no longer an admin's (the scheduled run uses `reeve decision check --strict`); each such decision keeps its last result. 2: the server refused the token, the token is not an admin's, another run still holds the lock, or the script refused to start (no token, a repository name that is not one directory name, a clone that failed). 124 (137 if it had to be killed): the run took longer than `REEVE_CHECK_TIMEOUT` (default 480 seconds; set it in the server's compose `environment:`, below the unit's `TimeoutStartSec=9min`) and was stopped with every process it started. Any other status comes from `reeve-server repos` (a configuration that does not parse) or from compose itself (the server container is not running).

Runs never overlap: each holds a lock on `.decision-check.lock` in the household volume until its last process is gone. Anything a run starts that detaches and outlives it holds the lock too, past the deadline. The script therefore runs git's own housekeeping (the `gc --auto` or `maintenance run --auto` a fetch starts) in the foreground (`gc.autoDetach=false`, `maintenance.autoDetach=false`, through `GIT_CONFIG_COUNT`, which the git that `reeve` runs inherits; added after any `GIT_CONFIG_*` settings the container already has, which are kept), so it stays inside the lock and the deadline; housekeeping is not switched off, so the household does not grow without bound. An ssh connection master (`ControlPersist`) would do the same, hence the advice above. A clone is made in `.clone-<name>` and renamed into place only when complete, so an interrupted run never leaves a half clone; the next run removes the leftover and clones again.

"Not found" is stored only when `origin/HEAD` resolved and the file is absent from that commit's tree; with origin unreachable the record is reported unreadable instead. A file that exists there but whose front matter cannot be parsed is stored as not accepted (status "unparsable front matter"), because that is a fact about the record. `decision link` accepts only repository names listed under `[[repos]]`; an empty list rejects every link, so configure the repositories before linking records.

### By hand, outside the container

Any machine with `reeve`, git and a household can run the same check, with an admin's automation token:

    REEVE_URL=https://planner.example.com REEVE_TOKEN=<automation token> reeve decision check --household /path/to/household

The household directory must hold a real `git clone` of each repository named in `reeve.toml` `[[repos]]`, at `<household>/<repo-name>`, with `origin/HEAD` set (`git remote set-head origin -a`), and git credentials that work without a prompt. A host that is not set up this way cannot fetch or read the record; the checker then stores nothing for those decisions (each keeps its last result), reports them as `unreadable` and exits 1, so the misconfiguration is noticed without flipping accepted decisions back to blocked.

## Backup and restore

The restore source is a `pg_dump` archive of the planner's database. It holds everything: users, token hashes (people stay signed in and automation keeps working after a restore), idempotency keys (a create retried after the restore still deduplicates), the key sequence, the activity log with its ids, and the migration table. Store a dump like a secret: it holds every user's email and every token hash. The JSON export is the readable record taken beside it, and what a restore is checked against; nothing restores from the export.

### Releases and rollback

Migration 1009 adds the urgent-mark columns to the initiative, epic and story tables. A rollback past the release that carries it therefore restores the pre-release backup (see "Restore after an incident"), and urgent marks made since that backup are lost. The image from before this release has no `reeve-server export --at` (its export computes no urgency, so there is nothing to pin to an instant): with that image, the step 5 check runs `export` without `--at`, as its snippet does when `export --help` does not list it, and `scripts/restore-rehearsal.sh` probes the same way.

### Back up

On the host, in the directory holding `docker-compose.yml`. This is the command the host's backup script runs:

    scripts/backup.sh /var/backups/reeve

It writes `reeve-<stamp>.dump` (`pg_dump -U reeve -d reeve --format=custom --no-owner` in the postgres container) and `reeve-<stamp>.json` (`reeve-server export` in the server container) into the directory and prints their paths. Both are written to fresh temporary files created with mode 0600 and get their final names only when both commands succeeded and the archive read back whole (`pg_restore -f /dev/null` reads every entry; the script also checks the `PGDMP` header, which says only that the file starts like an archive); nothing at a final name is ever overwritten. Exit status 0: both published. 2: nothing was published (a step failed, the export came back empty, the stamp was not letters and digits, or a file of that name appeared meanwhile); the message says which. Treat anything but 0 as "no backup was taken". The published pair is hard-linked from the temporaries, so the backup directory must be on a filesystem that supports hard links (a local ext4 or xfs disk does; some network and FUSE mounts do not, and the script then exits 2). A `kill -9` (or a power cut) of the script skips its cleanup and can leave hidden `.reeve-<stamp>.dump.*` and `.reeve-<stamp>.json.*` temporaries (mode 0600, never a final name) in the directory: delete them by hand, and prune them out of any sync that copies hidden files. Every `docker compose exec` in this section has `-T`: there is no terminal, and a terminal would corrupt the binary archive.

`pg_dump` reads one consistent snapshot and does not block the running server's reads or writes; do not run `reeve-server migrate` or upgrade the server while a dump runs (the schema change waits for the dump, or the dump fails). `reeve-server export` reads the database directly, needs no token, never writes, and refuses a database whose migrations differ from the binary's.

Without compose, with a PostgreSQL 16 client and the password in `~/.pgpass` rather than in the URL (keep it out of the process list), the same steps by hand:

    set -e    # stop at the first failing command: no export after a failed dump
    umask 077
    stamp=$(date -u +%Y%m%dT%H%M%SZ)
    tmp=$(mktemp ".reeve-$stamp.dump.XXXXXX")
    trap 'rm -f "$tmp"' EXIT
    pg_dump --format=custom --no-owner -d "<database_url>" > "$tmp"
    pg_restore -f /dev/null < "$tmp"
    ln -T "$tmp" "reeve-$stamp.dump"
    reeve-server --config <reeve.toml> export --out "reeve-$stamp.json"

`ln -T` refuses any existing name and runs only when `pg_dump` succeeded and the archive read back whole; `--out` writes a fresh 0600 file and renames it into place, replacing a file of that name that exists already.

### Restore after an incident

Anything written after the dump is lost; the restored activity log's last entry says how far it reaches. Use a dump `scripts/backup.sh` published (exit status 0), and restore with the server image that took it: upgrade afterwards, as a separate step, so the check in step 5 compares like with like. The server stays stopped until step 7, so no request or webhook changes the restored data before it is checked. Pause the scheduled backup for the duration of the restore, so it does not take a dump of a half-restored database, and stop the scheduled decision check (step 2 to step 8), which writes to the planner every ten minutes: while the server is stopped each run fails, and once it starts again a run would write to the restored instance before it is confirmed. The dump holds the database only: `reeve.toml` (the OIDC secrets, the webhook secret, the database password) is not in it, so keep a copy of it separately as a secret; the configured admins decide the roles in the export, so a different `reeve.toml` makes the step 5 comparison differ. The same file is the access floor (see "Access"): a grant in the dump that the restored floor no longer allows is kept, marked `not allowed` and inert, until an admin revokes it. To get the image that took the dump, check out the commit that was deployed at the time and run `docker compose build`.

1. Read the whole archive before anything is dropped: `docker compose exec -T postgres pg_restore -f /dev/null < reeve-<stamp>.dump` (a shell redirect or a pipe, the same) must exit 0. It reads every entry, table data included, into no database, and fails on a truncated or foreign file. (`pg_restore --list` is not enough: it reads only the header and the table of contents, and an archive cut in its table data passes it.)
2. `sudo systemctl stop reeve-decision-check.timer` (and wait for a run in progress: wait while `systemctl is-active reeve-decision-check.service` says `activating`; once the run is over it says `inactive`, or `failed` if the last run failed, as it is likely to while the server is broken), then `docker compose stop server`.
3. Keep the live database until the restore is confirmed: `docker compose exec -T postgres psql -U reeve -d postgres -c "ALTER DATABASE reeve RENAME TO reeve_before_restore"`, then `docker compose exec -T postgres createdb -U reeve -O reeve reeve`. The rename refuses while anything is connected, and when `reeve_before_restore` exists already (a leftover of an earlier restore: drop it first only if you are sure you no longer need it); `docker compose exec -T postgres psql -U reeve -d postgres -c "SELECT pid, application_name, client_addr FROM pg_stat_activity WHERE datname = 'reeve'"` shows who is connected. The kept copy needs disk for two databases until step 8.
4. `docker compose exec -T postgres pg_restore -U reeve -d reeve --no-owner --exit-on-error < reeve-<stamp>.dump`. It stops at the first error instead of restoring part of the database. After a failure, drop only the new, half-restored database (`docker compose exec -T postgres dropdb -U reeve reeve`; `reeve_before_restore` stays) and go back to `createdb` in step 3: a second restore into a database that is no longer empty fails on the first table that exists. If the restore cannot be made to work, drop the new `reeve` and rename `reeve_before_restore` back to `reeve`.
5. Check the restored data with the server still stopped: `docker compose run` starts a one-off server container that publishes no port and runs one command.

       restored=$(mktemp)    # a fresh file, mode 0600: the export holds every user's email
       at=$(sed -n 's/^  "exported_at": "\(.*\)",$/\1/p' reeve-<stamp>.json)    # urgency is computed at exported_at
       at_arg=(--at "$at")    # an image from before urgent marks has no --at (and computes no urgency): omit it
       docker compose run --rm --no-deps -T server export --help | grep -q -- '--at ' || at_arg=()
       docker compose run --rm --no-deps -T server export "${at_arg[@]}" > "$restored"
       diff <(grep -v '^  "exported_at": ' reeve-<stamp>.json) <(grep -v '^  "exported_at": ' "$restored") && echo restored data matches
       rm -f "$restored"

   It prints `restored data matches` when the restored instance holds the same data as the export taken with the dump. An entry written between the dump and its export shows up as a difference. If `export` refuses with "run `reeve-server migrate` first", the image is newer than the dump: run step 6 first, then this check, and expect only the fields the newer server adds. The comparison needs only `grep` and `diff`; `jq` is not required.
6. `docker compose run --rm --no-deps server migrate` (a newer image applies only the migrations newer than the dump; the same image applies nothing).
7. `docker compose start server`.
8. Once the restored instance is confirmed (people sign in, the board shows the expected stories), drop the kept copy: `docker compose exec -T postgres dropdb -U reeve reeve_before_restore`, and start the decision check again: `sudo systemctl start reeve-decision-check.timer`.

Without compose (a PostgreSQL 16 client; the password in `~/.pgpass`, and a `<database_url>` without it, so it stays out of the process list): read the whole archive with `pg_restore -f /dev/null < reeve-<stamp>.dump`, stop the server, `psql -d postgres -c "ALTER DATABASE <name> RENAME TO <name>_before_restore"` and `createdb -O <owner> <name>`, `pg_restore -d "<database_url>" --no-owner --exit-on-error < reeve-<stamp>.dump`, run `reeve-server --config <reeve.toml> export --out <fresh name>` and compare as in step 5, run `reeve-server migrate`, start the server, and drop `<name>_before_restore` once the restored instance is confirmed (it needs disk for two copies until then).

### Rehearse the restore

Before cut-over, the operator and then a second named person each run this once on the real host. It restores into a scratch database beside the live one and never touches `reeve`.

1. Stop the scheduled decision check, which writes every ten minutes and would otherwise land between the dump and the export: `sudo systemctl stop reeve-decision-check.timer` (and wait while `systemctl is-active reeve-decision-check.service` says `activating`: a run is under way; `inactive` or `failed` means none is). At a quiet moment, take a backup (`scripts/backup.sh <directory>`), then start the timer again: `sudo systemctl start reeve-decision-check.timer`.
2. Run `scripts/restore-rehearsal.sh <directory>/reeve-<stamp>.dump <directory>/reeve-<stamp>.json` from the repository directory holding `docker-compose.yml`. It creates `reeve_rehearsal` (it refuses when that database exists, and refuses the name `reeve`; `REEVE_REHEARSAL_DB` picks another name), restores the dump into it, exports it offline, compares it with the backup's export apart from `exported_at`, and drops the scratch database on exit, also after a failure. The offline export runs with `--at` set to the backup export's `exported_at`: urgency is computed at `exported_at` (an urgent mark is active for one cycle after it was set), so the restored document is computed at the same instant and a mark that expired since the backup does not show as a difference. A server image from before urgent marks has no `--at`: the script reads `export --help` first and leaves `--at` out when it is not listed (that image computes no urgency). Exit status 0: the documents match and the scratch database is gone. 1: they differ, and the diff is printed; an entry written between the dump and the export is the only acceptable difference, so take the backup again at a quieter moment. 2: refused, or a step failed, the final drop included (the message says which; drop `reeve_rehearsal` by hand if it says so).
3. Record the date, the dump's file name and who ran it in the team's runbook log.

By hand, the script runs:

    docker compose exec -T postgres createdb -U reeve -O reeve reeve_rehearsal
    docker compose exec -T postgres pg_restore -U reeve -d reeve_rehearsal --no-owner --exit-on-error < reeve-<stamp>.dump
    restored=$(mktemp)    # a fresh file, mode 0600
    at=$(sed -n 's/^  "exported_at": "\(.*\)",$/\1/p' reeve-<stamp>.json)
    docker compose exec -T server reeve-server export --database reeve_rehearsal --at "$at" > "$restored"
    diff <(grep -v '^  "exported_at": ' reeve-<stamp>.json) <(grep -v '^  "exported_at": ' "$restored")
    rm -f "$restored"
    docker compose exec -T postgres dropdb -U reeve reeve_rehearsal

`--database` reads another database on the server the configuration names, with the configuration's user and password, so no password goes on the command line. The rehearsal runs beside the live server (`exec`, not `run`): it reads the live database only through the dump.
