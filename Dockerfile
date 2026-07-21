ARG LIGHTMONITOR_VERSION=1.0.5

FROM rust:1.96-bookworm AS rust-build
WORKDIR /src

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --workspace

FROM node:24-bookworm AS web-build
WORKDIR /web

COPY web/package*.json ./
RUN npm ci
COPY web ./
RUN npm run build

FROM debian:bookworm-slim
ARG LIGHTMONITOR_VERSION
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl openssh-client sshpass \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=rust-build /src/target/release/server /app/bundled/lightmonitor-server
COPY --from=rust-build /src/target/release/agent /app/releases/lightmonitor-agent-linux-x86_64
COPY --from=web-build /web/dist /app/bundled/web
COPY scripts/launch-server.sh /usr/local/bin/lightmonitor-launcher
RUN chmod +x /app/bundled/lightmonitor-server /usr/local/bin/lightmonitor-launcher \
  && printf '%s\n' "$LIGHTMONITOR_VERSION" > /app/bundled/VERSION

ENV HOST=0.0.0.0
ENV PORT=8080
ENV LIGHTMONITOR_DATA_DIR=/app/data
ENV LIGHTMONITOR_WEB_DIR=/app/bundled/web
ENV LIGHTMONITOR_RELEASES_DIR=/app/releases
ENV LIGHTMONITOR_VERSIONS_DIR=/app/data/versions
ENV LIGHTMONITOR_MANAGED_UPDATES=true
ENV LIGHTMONITOR_ADMIN_USERNAME=admin
ENV LIGHTMONITOR_ADMIN_PASSWORD=admin

VOLUME ["/app/data"]
EXPOSE 8080

CMD ["lightmonitor-launcher"]
