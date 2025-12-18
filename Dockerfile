ARG RUST_VERSION=1.92.0

FROM rust:${RUST_VERSION}-bookworm AS builder
WORKDIR /usr/src/haunted-coop
COPY . .
RUN apt update \
 && apt install -yq pkg-config libssl-dev \
 && cargo test \
 && cargo install --path . --locked

FROM rust:${RUST_VERSION}-slim-bookworm
COPY --from=builder /usr/local/cargo/bin/haunted-coop /usr/local/bin/haunted-coop
EXPOSE 1996
USER 1000
ENTRYPOINT ["haunted-coop"]
