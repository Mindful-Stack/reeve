# The client is built by Node in its own stage; the runtime image has no Node (kb/ADR-0006 R1).
FROM node:22-bookworm-slim AS web
WORKDIR /src
# Manifests first so the npm layer survives source edits. `npm run build` does not regenerate types (schema.d.ts is
# committed and CI checks it against the snapshot); the snapshot is copied so `npm run gen` works in this stage too.
COPY web/package.json web/package-lock.json web/.npmrc web/
RUN cd web && npm ci
COPY web web/
COPY crates/reeve-server/tests/snapshots/openapi.json crates/reeve-server/tests/snapshots/
RUN cd web && npm run build && test -f build/index.html

# cargo-chef splits the Rust build: `deps` compiles the dependencies from the manifests and the lock file alone, so
# its layer survives every source edit and comes from the image cache (release.yml writes it on every push to main).
FROM rust:1.99-bookworm AS chef
RUN cargo install cargo-chef --locked --version 0.1.78
WORKDIR /src

# rust-toolchain.toml names the floating "stable" channel (and RUSTUP_TOOLCHAIN=1.99.0 is not the image's 1.99.1
# toolchain, so rustup would download one). cargo-chef records the file in the recipe and writes it back during
# `cook`, so it goes before `prepare`, and again after the source is copied into `build`: the image's toolchain
# builds everything.
FROM chef AS planner
COPY . .
RUN rm rust-toolchain.toml && cargo chef prepare --recipe-path recipe.json

# The final build's packages and profile; never static-oidc.
FROM chef AS deps
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked -p reeve-server -p reeve --recipe-path recipe.json

FROM deps AS build
COPY . .
# web/build is dockerignored; the embedded client comes from the web stage.
COPY --from=web /src/web/build /src/web/build
# An empty web/build compiles fine (build.rs creates it) and serves 503 on every page; refuse to ship that.
RUN test -f web/build/index.html
# target/ is deleted in the same layer, so the layer that changes with every commit holds two binaries, not the
# target directory; `deps` keeps the dependencies' artifacts.
RUN rm rust-toolchain.toml && cargo build --release --locked -p reeve-server -p reeve \
    && mkdir /out && cp target/release/reeve-server target/release/reeve /out/ && rm -rf target
FROM debian:bookworm-slim
# git and an ssh client for the scheduled decision check (deploy/decision-check.sh), which clones and fetches the
# household inside this container; age for an operator's encrypted off-host backups and their restore rehearsal, run
# as one-off containers of this image. The server itself runs none of them.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates git openssh-client age && rm -rf /var/lib/apt/lists/*
COPY --from=build /out/reeve-server /usr/local/bin/
COPY --from=build /out/reeve /usr/local/bin/
COPY deploy/decision-check.sh /usr/local/bin/reeve-decision-check
RUN chmod 0755 /usr/local/bin/reeve-decision-check
EXPOSE 7474
ENTRYPOINT ["reeve-server"]
CMD ["serve"]
