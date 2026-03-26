FROM rust:1.85-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates curl && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/oidc-exchange /usr/local/bin/
COPY examples/container/config/ /app/config/
WORKDIR /app
EXPOSE 8080
ENV OIDC_EXCHANGE_ENV=production
CMD ["oidc-exchange"]
