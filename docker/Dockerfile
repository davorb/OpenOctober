# Runtime image only: the source, the game assets and the databases all live on the host and are mounted in (see ../docker-compose.yml).
# The game-server is compiled by the entrypoint at every start, so a `git pull` on the host is all it takes to update it.
FROM rust:1-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends nodejs tzdata && rm -rf /var/lib/apt/lists/*
ENV TZ=Europe/Stockholm \
    CARGO_TARGET_DIR=/cache/target
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh
WORKDIR /app
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
