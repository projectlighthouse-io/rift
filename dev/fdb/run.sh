#!/bin/sh
# Runs a cargo command for the FoundationDB driver inside a Linux container that shares the
# Docker FoundationDB network, so 127.0.0.1:4500 reaches the cluster.
# Usage: dev/fdb/run.sh test -p foundationdb-driver
set -e
cd "$(dirname "$0")/../.."
docker build -q -t rift-fdb-dev dev/fdb >/dev/null
fdb=$(docker compose ps -q foundationdb)
[ -n "$fdb" ] || { echo "FoundationDB is not running: make dev-up" >&2; exit 1; }
docker run --rm --network "container:$fdb" \
  -e RIFT_FDB_CLUSTER=docker:docker@127.0.0.1:4500 -e RIFT_REQUIRE_DOCKER \
  -v "$PWD":/rift -w /rift \
  -v rift-fdb-target:/rift/target -v rift-fdb-cargo:/usr/local/cargo/registry \
  rift-fdb-dev cargo "$@"
