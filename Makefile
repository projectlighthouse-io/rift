.PHONY: dev-up dev-seed dev-down

# Start local databases and wait until they are healthy.
dev-up:
	docker compose up -d --wait

# Load seed data. Safe to re-run: each script resets what it owns.
dev-seed:
	docker compose exec -T postgres psql -v ON_ERROR_STOP=1 -U postgres -d shop_dev -f /seed/postgres.sql
	docker compose exec -T redis sh /seed/redis.sh
	docker compose exec -T mysql sh -c "mysql -uroot -pmysql < /seed/mysql.sql"
	docker compose exec -T clickhouse clickhouse-client --password clickhouse --multiquery --queries-file /seed/clickhouse.sql

# Stop databases and delete their data.
dev-down:
	docker compose down -v
