.PHONY: dev-up dev-seed dev-down fdb-test samply

# Start local databases and wait until they are healthy.
dev-up:
	@mkdir -p dev/ssh/keys && test -f dev/ssh/keys/id_ed25519 || ssh-keygen -q -t ed25519 -N '' -f dev/ssh/keys/id_ed25519
	docker compose up -d --wait
	docker compose exec -T foundationdb sh -c 'fdbcli --exec "status minimal" --timeout 5 | grep -q available || fdbcli --exec "configure new single memory" --timeout 10'

# Load seed data. Safe to re-run: each script resets what it owns.
dev-seed:
	docker compose exec -T postgres psql -v ON_ERROR_STOP=1 -U postgres -d shop_dev -f /seed/postgres.sql
	docker compose exec -T redis sh /seed/redis.sh
	docker compose exec -T mysql sh -c "mysql -uroot -pmysql < /seed/mysql.sql"
	docker compose exec -T clickhouse clickhouse-client --password clickhouse --multiquery --queries-file /seed/clickhouse.sql
	dev/fdb/run.sh test -p foundationdb-driver --lib -- --ignored seed_dev_data --nocapture

# FoundationDB driver tests in a Linux container (no libfdb_c needed on the host).
fdb-test:
	RIFT_REQUIRE_DOCKER=1 dev/fdb/run.sh test -p foundationdb-driver

# Stop databases and delete their data.
dev-down:
	docker compose down -v

# --- Profiling (see docs/DECISIONS.md, "Profiling") ---------------------------------------------

# CPU flamegraph of a release-speed build, opened in the Firefox Profiler when rift quits.
# First time only: `samply setup` lets samply attach to processes on macOS.
samply:
	cargo build --profile profiling -p rift
	samply record target/profiling/rift
