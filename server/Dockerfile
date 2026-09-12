# syntax=docker/dockerfile:1.7
# Stage 1: static musl build. rust:alpine ships musl-native cargo, so no cross toolchain is needed.
FROM rust:1-alpine AS build
# hadolint ignore=DL3018
RUN apk add --no-cache musl-dev=~1.2 build-base=~0.5 perl=~5.40 make=~4.4 || apk add --no-cache musl-dev build-base perl make
WORKDIR /src/server
# Pre-build dependencies for layer caching.
COPY server/Cargo.toml server/Cargo.lock ./
RUN mkdir -p src ../web/dist && echo 'fn main(){}' > src/main.rs && echo '' > src/lib.rs \
    && echo '<html></html>' > ../web/dist/index.html \
    && cargo build --release --locked && rm -rf src target/release/deps/kube_docs_mcp* target/release/kube-docs-mcp*
COPY server/ ./
COPY web/dist ../web/dist
RUN cargo build --release --locked && strip target/release/kube-docs-mcp

# Stage 2: CA bundle for outbound TLS (index download).
FROM alpine:3.22 AS certs
# hadolint ignore=DL3018
RUN apk add --no-cache ca-certificates=~20250619 || apk add --no-cache ca-certificates

# Stage 3: scratch runtime.
FROM scratch
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build /src/server/target/release/kube-docs-mcp /kube-docs-mcp
ENV KD_INDEX=/data/index.sqlite KD_ALLOWED_HOSTS=localhost:8080 SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
VOLUME ["/data"]
USER 65534:65534
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=120s --retries=3 CMD ["/kube-docs-mcp","healthcheck"]
ENTRYPOINT ["/kube-docs-mcp","serve","--http","0.0.0.0:8080","--index","/data/index.sqlite"]
