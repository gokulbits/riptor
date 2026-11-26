# syntax=docker/dockerfile:1
# Headless Riptor (riptord + riptor-cli) with remote control and the web interface.
# python3 runs search plugins; see README.md, "Docker".

FROM rust:1-slim-trixie AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libdbus-1-dev cmake make perl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p riptor-daemon \
    && mkdir /out \
    && cp target/release/riptord target/release/riptor-cli /out/

FROM debian:trixie-slim
ARG DEBIAN_FRONTEND=noninteractive
# setpriv is split out of util-linux on newer Debian; either way it has to be there.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libdbus-1-3 tzdata python3 \
    && (apt-get install -y --no-install-recommends setpriv || true) \
    && command -v setpriv \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /out/ /usr/local/bin/
COPY --chmod=755 docker/entrypoint.sh /usr/local/bin/riptor-entrypoint

ENV HOME=/config \
    PUID=1000 \
    PGID=1000 \
    UMASK=022 \
    RIPTOR_SOCKET_DIR=/tmp/riptor \
    RIPTOR_DOWNLOAD_DIR=/downloads \
    RIPTOR_PEER_PORT=6881 \
    RIPTOR_WEBUI=on \
    RIPTOR_WEBUI_PORT=8080

VOLUME ["/config", "/downloads"]
EXPOSE 8080/tcp 6881/tcp 6881/udp
HEALTHCHECK --interval=30s --timeout=10s --start-period=60s --retries=3 \
    CMD riptor-cli stats > /dev/null || exit 1
ENTRYPOINT ["riptor-entrypoint"]
