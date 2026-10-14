# Runtime image only: the source, your own game files and the databases live on the host and are mounted in (see ../docker-compose.yml).
# The game-server is compiled by the entrypoint at every start, so pulling new source on the host is all it takes to update it.
FROM rust:1-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends nodejs && rm -rf /var/lib/apt/lists/*
ENV CARGO_TARGET_DIR=/cache/target
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh
WORKDIR /app
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
