FROM rust:1.61-slim as builder
WORKDIR /usr/src/haunted-coop
COPY . .
RUN cargo test && cargo install --path . --locked

FROM rust:1.60-slim-buster
COPY --from=builder /usr/local/cargo/bin/haunted-coop /usr/local/bin/haunted-coop
EXPOSE 1996
ENTRYPOINT ["haunted-coop"]
