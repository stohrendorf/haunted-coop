FROM rust:1.60-slim as builder
WORKDIR /usr/src/haunted-coop
COPY . .
RUN cargo test && cargo install --path .

FROM rust:1.60-slim-buster
# RUN apt-get update && apt-get install -y extra-runtime-dependencies && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/haunted-coop /usr/local/bin/haunted-coop
EXPOSE 1996
CMD ["haunted-coop"]
