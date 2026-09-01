# Materialize the filtered context once and make its security boundary executable. CI plants the
# sentinel before building this stage; a permissive `.dockerignore` would make the build fail.
FROM debian:bookworm-slim AS context-audit
WORKDIR /context
COPY . .
RUN test -f Cargo.toml \
    && test -f crates/catcoms-app/src/store.rs \
    && test -f apps/desktop/src/App.svelte \
    && test -f assets/mewtual-logo.svg \
    && test -f assets/cat/mascot-idle.svg \
    && test ! -e .claude/review-secret \
    && test ! -e .npmrc \
    && test ! -e .git \
    && test ! -e .agents \
    && test ! -e .codex

# A test environment, not a distributable desktop image. Debian supplies GTK/WebKitGTK so the
# Linux Tauri workspace is compiled against the same class of native libraries used in a desktop
# install. Screen/audio portal behavior still requires a real graphical login and is deliberately
# outside this headless container's claims; see docs/LINUX-TESTING.md.
FROM node:22-bookworm-slim AS node-runtime

FROM rust:1.89-bookworm

ARG MEWTUAL_TEST_UID=1000
ARG MEWTUAL_TEST_GID=1000

# The frontend dependency set requires Node >=20 (`marked`); Debian Bookworm's apt package is 18.
# Copy the official Node 22 runtime rather than running a third-party repository bootstrap script.
COPY --from=node-runtime /usr/local/ /usr/local/

ENV DEBIAN_FRONTEND=noninteractive \
    CARGO_TERM_COLOR=always \
    RUSTFLAGS=-Dwarnings

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        git \
        iproute2 \
        iputils-ping \
        libasound2-dev \
        libayatana-appindicator3-dev \
        librsvg2-dev \
        libssl-dev \
        libwebkit2gtk-4.1-dev \
        nftables \
        patchelf \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add rustfmt clippy

RUN groupadd --gid "${MEWTUAL_TEST_GID}" mewtual \
    && useradd --create-home --uid "${MEWTUAL_TEST_UID}" --gid "${MEWTUAL_TEST_GID}" \
        --shell /bin/bash mewtual

WORKDIR /workspace
COPY --from=context-audit --chown=mewtual:mewtual /context/ .

# The default/full lane compiles hostile-input boundaries but needs no root or kernel-admin
# capability. Compose explicitly opts the separate network-namespace lanes back into uid 0.
USER mewtual

CMD ["bash", "scripts/linux-container-test.sh", "full", "--install"]
