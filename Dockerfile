# Multi-stage build for the WebSocket relay server.
# Only `server` (and its crdt-core dependency) is built - client-wasm targets
# wasm32 and has no business in this image.

FROM rust:1-slim-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN cargo build --release -p server

FROM debian:bookworm-slim
# The server does no outbound TLS and touches no filesystem, so the runtime image
# needs nothing beyond the binary itself.
COPY --from=builder /app/target/release/server /usr/local/bin/server

ENV PORT=8080
EXPOSE 8080

CMD ["server"]
