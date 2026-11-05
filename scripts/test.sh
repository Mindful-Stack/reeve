#!/usr/bin/env bash
# One PostgreSQL 16 for the whole test run;
# every test clones a database of its own from it (crates/reeve-testdb).
# With arguments, one nextest run with exactly them
# (`mise run acceptance` and its siblings pass $REEVE_FEATURES themselves);
# without, the suite: the workspace with the static provider, the doc tests,
# and config_no_static without it.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
# The same settings as reeve_testdb::SETTINGS (its tests check this line).
settings=(fsync=off synchronous_commit=off full_page_writes=off max_connections=1000 shared_preload_libraries=pg_stat_statements pg_stat_statements.max=100000)
# mise.toml's [env] normally sets REEVE_FEATURES; an explicitly empty value is kept.
read -ra features <<<"${REEVE_FEATURES---features reeve-server/static-oidc}"

args=()
for s in "${settings[@]}"; do args+=(-c "$s"); done
cid=""
trap '[ -z "$cid" ] || docker rm -f "$cid" >/dev/null 2>&1 || true' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
cid=$(docker run -d --rm --label reeve.test-run=1 -e POSTGRES_PASSWORD=postgres -p 127.0.0.1::5432 postgres:16 postgres "${args[@]}")
# The image's entrypoint runs a socket-only server while it initialises, so a TCP answer means the real one is up.
ready=""
for _ in $(seq 120); do
	if docker exec "$cid" pg_isready -q -h 127.0.0.1 -U postgres; then ready=1; break; fi
	sleep 0.5
done
if [ -z "$ready" ]; then
	docker logs "$cid" >&2 || true
	echo "test.sh: PostgreSQL did not accept connections within 60 s" >&2
	exit 1
fi
port=$(docker port "$cid" 5432/tcp | head -n1)
export REEVE_TEST_DATABASE_URL="postgres://postgres:postgres@127.0.0.1:${port##*:}/postgres"

if [ $# -gt 0 ]; then
	# The caller passes features itself: not every package has reeve-server's.
	cargo nextest run "$@"
else
	cargo nextest run --workspace "${features[@]}"
	cargo test --workspace --doc "${features[@]}"
	cargo nextest run -p reeve-server --test config_no_static
fi
