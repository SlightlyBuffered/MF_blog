FROM rust:1.82-bookworm AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim

ARG APP_UID=1000
ARG APP_GID=1000

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid "${APP_GID}" mfblog \
    && useradd --uid "${APP_UID}" --gid "${APP_GID}" --create-home mfblog

WORKDIR /app

COPY --from=builder /build/target/release/mf-blog /usr/local/bin/mf-blog
COPY site.yaml ./
COPY templates ./templates
COPY static ./static
COPY docs ./docs

RUN mkdir -p /app/articles /data \
    && chown -R mfblog:mfblog /app /data

USER mfblog

ENV BLOG_BIND=0.0.0.0:8000

EXPOSE 8000
VOLUME ["/data"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl --fail --silent --max-time 3 http://127.0.0.1:8000/ >/dev/null || exit 1

CMD ["mf-blog"]
