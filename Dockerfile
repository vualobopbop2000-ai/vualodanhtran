FROM rust:1.84-slim AS builder
WORKDIR /app
COPY Cargo.toml ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/emperors-great-battle /usr/local/bin/emperors-great-battle
COPY public ./public
ENV PORT=10000
ENV RUST_LOG=info
EXPOSE 10000
CMD ["/usr/local/bin/emperors-great-battle"]
