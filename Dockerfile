FROM python:3.12-slim

ARG APP_UID=1000
ARG APP_GID=1000

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1

RUN groupadd --gid "${APP_GID}" mfblog \
    && useradd --uid "${APP_UID}" --gid "${APP_GID}" --create-home mfblog

WORKDIR /app

COPY pyproject.toml README.md ./
COPY app.py site.yaml ./
COPY templates ./templates
COPY static ./static
COPY docs ./docs

RUN mkdir -p /app/articles /data \
    && chown -R mfblog:mfblog /app /data \
    && python -m pip install --no-cache-dir ".[deploy]"

USER mfblog

EXPOSE 8000
VOLUME ["/data"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD python -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8000/', timeout=3)" || exit 1

CMD ["gunicorn", "--bind", "0.0.0.0:8000", "--workers", "1", "--threads", "4", "--access-logfile", "-", "app:app"]
