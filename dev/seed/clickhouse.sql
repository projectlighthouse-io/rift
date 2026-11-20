-- Seed for ClickHouse `analytics`: ~4M events over 14 days. Re-runnable.
DROP DATABASE IF EXISTS analytics;
CREATE DATABASE analytics;

CREATE TABLE analytics.events
(
    ts DateTime,
    event LowCardinality(String),
    user_id UInt64,
    country LowCardinality(String)
)
ENGINE = MergeTree
PARTITION BY toDate(ts)
ORDER BY (event, ts)
TTL ts + INTERVAL 90 DAY;

INSERT INTO analytics.events
SELECT
    now() - toIntervalSecond(rand(1) % (14 * 86400)) AS ts,
    arrayElement(['page_view', 'page_view', 'page_view', 'click', 'add_to_cart', 'checkout', 'signup'], 1 + rand(2) % 7) AS event,
    1 + rand(3) % 50000 AS user_id,
    arrayElement(['GB', 'GB', 'US', 'US', 'DE', 'FR', 'IE', 'NL'], 1 + rand(4) % 8) AS country
FROM numbers(4000000);

CREATE TABLE analytics.page_views
(
    ts DateTime,
    path String,
    user_id UInt64,
    duration_ms UInt32
)
ENGINE = MergeTree
ORDER BY ts;

INSERT INTO analytics.page_views
SELECT now() - toIntervalSecond(rand(5) % (14 * 86400)),
       arrayElement(['/', '/products', '/cart', '/checkout', '/account'], 1 + rand(6) % 5),
       1 + rand(7) % 50000, rand(8) % 30000
FROM numbers(500000);

CREATE TABLE analytics.sessions
(
    started_at DateTime,
    user_id UInt64,
    pages UInt16,
    converted UInt8
)
ENGINE = MergeTree
ORDER BY started_at;

INSERT INTO analytics.sessions
SELECT now() - toIntervalSecond(rand(9) % (14 * 86400)), 1 + rand(10) % 50000, 1 + rand(11) % 20, rand(12) % 10 = 0
FROM numbers(200000);

CREATE TABLE analytics.errors
(
    ts DateTime,
    service LowCardinality(String),
    message String
)
ENGINE = MergeTree
ORDER BY ts
TTL ts + INTERVAL 30 DAY;

INSERT INTO analytics.errors
SELECT now() - toIntervalSecond(rand(13) % (14 * 86400)),
       arrayElement(['api', 'web', 'worker'], 1 + rand(14) % 3),
       arrayElement(['timeout', 'not found', 'rate limited'], 1 + rand(15) % 3)
FROM numbers(20000);

CREATE MATERIALIZED VIEW analytics.events_daily_mv
ENGINE = SummingMergeTree
ORDER BY (day, event)
POPULATE
AS SELECT toDate(ts) AS day, event, count() AS events
FROM analytics.events
GROUP BY day, event;
