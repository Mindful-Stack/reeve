# The CLI bundle as an image: the binaries, SHA256SUMS and VERSION at its root and nothing else (kb/ADR-0030 R1).
# cli.yml builds it with the bundle directory as the context and pushes it once per CLI version as
# ghcr.io/mindful-stack/reeve-cli:<version>; the server image copies it to /srv/cli. It never runs.
FROM scratch
# Links the package to this repository, so the repository's workflows can push and pull it.
LABEL org.opencontainers.image.source=https://github.com/Mindful-Stack/reeve
COPY . /
