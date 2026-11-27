-- Demo shop database for trying Rift without Docker.
--   sqlite3 shop.sqlite < dev/seed/sqlite.sql
-- Re-runnable: drops what it creates. Data is random but shaped like the Postgres seed.

PRAGMA foreign_keys = OFF;
DROP VIEW IF EXISTS orders_daily_summary;
DROP VIEW IF EXISTS customer_lifetime_value;
DROP TABLE IF EXISTS order_items;
DROP TABLE IF EXISTS orders;
DROP TABLE IF EXISTS products;
DROP TABLE IF EXISTS customers;
PRAGMA foreign_keys = ON;

CREATE TABLE customers (
  id INTEGER PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  country TEXT NOT NULL DEFAULT 'GB',
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE products (
  id INTEGER PRIMARY KEY,
  sku TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  price REAL NOT NULL,
  active INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE orders (
  id INTEGER PRIMARY KEY,
  customer_id INTEGER NOT NULL REFERENCES customers (id),
  status TEXT NOT NULL DEFAULT 'pending'
    CHECK (status IN ('pending', 'paid', 'shipped', 'delivered', 'cancelled', 'refunded')),
  total REAL NOT NULL DEFAULT 0,
  currency TEXT NOT NULL DEFAULT 'GBP',
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  shipped_at TEXT,
  notes TEXT
);
CREATE INDEX orders_customer_id_idx ON orders (customer_id);
CREATE INDEX orders_status_created_idx ON orders (status, created_at DESC);

CREATE TABLE order_items (
  id INTEGER PRIMARY KEY,
  order_id INTEGER NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
  product_id INTEGER NOT NULL REFERENCES products (id),
  quantity INTEGER NOT NULL CHECK (quantity > 0),
  unit_price REAL NOT NULL
);

WITH RECURSIVE g(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM g WHERE n < 500)
INSERT INTO customers (id, email, name, country, created_at)
SELECT n, 'customer' || n || '@example.com', 'Customer ' || n,
       json_extract('["GB","GB","GB","IE","DE","FR","US"]', '$[' || abs(random() % 7) || ']'),
       datetime('now', '-' || abs(random() % 700) || ' days')
FROM g;

WITH v(id, email, name) AS (VALUES
  (5, 'r.brennan@example.com', 'Rory Brennan'),
  (9, 'sam.w@example.com', 'Sam Walker'),
  (17, 't.nguyen@example.com', 'Thao Nguyen'),
  (62, 'kim.l@example.com', 'Kim Lee'),
  (88, 'jo.harper@example.com', 'Jo Harper'),
  (130, 'a.rossi@example.com', 'Alessia Rossi'),
  (204, 'lina.p@example.com', 'Lina Petrova'),
  (300, 'f.ahmed@example.com', 'Farah Ahmed'),
  (312, 'm.okafor@example.com', 'Maya Okafor'),
  (451, 'devi.k@example.com', 'Devi Kumar')
)
UPDATE customers SET email = v.email, name = v.name
FROM v WHERE customers.id = v.id;

WITH RECURSIVE g(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM g WHERE n < 120)
INSERT INTO products (id, sku, name, price, active)
SELECT n, 'SKU-' || printf('%04d', n), 'Product ' || n,
       round(5 + abs(random() % 29500) / 100.0, 2), abs(random() % 100) > 8
FROM g;

-- 1,284 orders over the last 60 days, ids increasing with time.
WITH RECURSIVE g(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM g WHERE n < 1284),
raw AS MATERIALIZED (
  SELECT 1 + abs(random() % 500) AS customer_id,
         json_extract('["pending","paid","paid","paid","shipped","shipped","delivered","delivered","delivered","cancelled","refunded"]',
                      '$[' || abs(random() % 11) || ']') AS status,
         round(5 + abs(random() % 32000) / 100.0, 2) AS total,
         json_extract('["GBP","GBP","GBP","GBP","EUR","USD"]', '$[' || abs(random() % 6) || ']') AS currency,
         datetime('now', '-' || abs(random() % 5184000) || ' seconds') AS created_at,
         CASE WHEN abs(random() % 100) < 6
              THEN json_extract('["Leave with neighbour","Gift wrap","Duplicate order","Damaged in transit","Call before delivery"]',
                                '$[' || abs(random() % 5) || ']')
         END AS notes
  FROM g
)
INSERT INTO orders (customer_id, status, total, currency, created_at, shipped_at, notes)
SELECT customer_id, status, total, currency, created_at,
       CASE WHEN status IN ('shipped', 'delivered')
            THEN datetime(created_at, '+' || abs(random() % 30) || ' hours') END,
       notes
FROM raw ORDER BY created_at;

INSERT INTO order_items (order_id, product_id, quantity, unit_price)
SELECT o.id, p.id, 1 + abs(random() % 3), p.price
FROM orders o JOIN products p ON p.id = 1 + (o.id * 7 + abs(random() % 3)) % 120;

CREATE VIEW orders_daily_summary AS
SELECT date(created_at) AS day, count(*) AS orders, round(sum(total), 2) AS revenue
FROM orders WHERE status <> 'cancelled'
GROUP BY 1;

CREATE VIEW customer_lifetime_value AS
SELECT c.id, c.email, count(o.id) AS orders, round(coalesce(sum(o.total), 0), 2) AS revenue
FROM customers c LEFT JOIN orders o ON o.customer_id = c.id AND o.status <> 'cancelled'
GROUP BY c.id, c.email;
