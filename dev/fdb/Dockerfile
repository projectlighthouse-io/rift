# Rust plus the FoundationDB client library, for running the FoundationDB driver's tests and
# seed on machines without libfdb_c (`make fdb-test`, `make dev-seed`).
FROM rust:1.98-slim
ARG FDB_VERSION=7.3.69
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates adduser \
 && arch=$(dpkg --print-architecture | sed 's/arm64/aarch64/') \
 && curl -fsSL -o /tmp/fdb.deb \
    "https://github.com/apple/foundationdb/releases/download/${FDB_VERSION}/foundationdb-clients_${FDB_VERSION}-1_${arch}.deb" \
 && dpkg -i /tmp/fdb.deb && rm /tmp/fdb.deb && rm -rf /var/lib/apt/lists/*
