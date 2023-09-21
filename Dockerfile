FROM rust:1.72-bookworm as builder
WORKDIR /usr/src/haunted-coop
COPY . .
RUN apt update && apt install -yq pkg-config libssl-dev && cargo test && cargo install --path . --locked

FROM rust:1.72-slim-bookworm
COPY --from=builder /usr/local/cargo/bin/haunted-coop /usr/local/bin/haunted-coop
EXPOSE 1996
USER 1000
ENTRYPOINT ["haunted-coop"]
