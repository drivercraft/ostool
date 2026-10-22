# 与 CI 使用相同的 Node.js 与 pnpm；Rust 构建脚本负责嵌入前后台页面。
FROM node:24-bookworm-slim AS node
FROM rust:1-bookworm AS builder
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s /usr/local/lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && npm install --global pnpm@10.33.0 \
    && apt-get update && apt-get install -y --no-install-recommends libudev-dev \
    && rm -rf /var/lib/apt/lists/*
ENV CARGO_BUILD_JOBS=4
WORKDIR /source
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/source/target \
    cargo build --locked --release -p ostool-server --bins \
    && mkdir /output \
    && cp target/release/ostool-server target/release/ostool-server-migrate-boards \
          target/release/ostool-server-seed-demo /output/

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libudev1 curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /var/lib/ostool-server ostool \
    && mkdir -p /var/lib/ostool-server && chown ostool:ostool /var/lib/ostool-server
COPY --from=builder /output/ /usr/local/bin/
COPY ostool-server/deploy/container.toml /etc/ostool-server/config.toml
USER ostool
WORKDIR /var/lib/ostool-server
ENTRYPOINT ["ostool-server"]
CMD ["--config", "/etc/ostool-server/config.toml", "--env-file", "/dev/null"]
