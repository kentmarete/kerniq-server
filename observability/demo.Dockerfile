# Builds examples/observability-demo. Context: the repository root.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p example-observability-demo \
 && cp target/release/observability-demo /observability-demo

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /observability-demo /usr/local/bin/observability-demo
USER 10001
EXPOSE 3000 9464
ENTRYPOINT ["/usr/local/bin/observability-demo"]
