# syntax=docker/dockerfile:1.7
FROM rust:1.98-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p silicon-browser-backend

FROM node:24-bookworm-slim AS runtime
ARG AGENT_BROWSER_VERSION=0.36.0
RUN npm install --global --allow-scripts=agent-browser "agent-browser@${AGENT_BROWSER_VERSION}" \
    && npm cache clean --force \
    && apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/silicon-browser-backend /usr/local/bin/silicon-browser-backend
RUN useradd --create-home --uid 10001 silicon-browser \
    && install --directory --owner=silicon-browser --group=silicon-browser /data
USER silicon-browser
WORKDIR /data
ENV PORT=8080 \
    SB_DATABASE_URL=sqlite:///data/silicon-browser.db?mode=rwc \
    AGENT_BROWSER_BIN=/usr/local/bin/agent-browser
EXPOSE 8080
VOLUME ["/data"]
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["node", "-e", "fetch('http://127.0.0.1:'+(process.env.PORT||'8080')+'/healthz').then(r=>{if(!r.ok)process.exit(1)}).catch(()=>process.exit(1))"]
ENTRYPOINT ["silicon-browser-backend"]
