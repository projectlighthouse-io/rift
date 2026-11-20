-- Seed for MySQL `shop`, mirroring the Postgres shop_dev schema. Re-runnable.
DROP DATABASE IF EXISTS shop;
CREATE DATABASE shop;
USE shop;
SET SESSION cte_max_recursion_depth = 100000;

CREATE TABLE users (
  id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
  email VARCHAR(255) NOT NULL UNIQUE,
  role VARCHAR(32) NOT NULL DEFAULT 'staff',
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE customers (
  id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
  email VARCHAR(255) NOT NULL UNIQUE,
  name VARCHAR(255) NOT NULL,
  country CHAR(2) NOT NULL DEFAULT 'GB',
  vip TINYINT(1) NOT NULL DEFAULT 0,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE addresses (
  id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
  customer_id BIGINT UNSIGNED NOT NULL,
  line1 VARCHAR(255) NOT NULL,
  city VARCHAR(128) NOT NULL,
  postcode VARCHAR(16) NOT NULL,
  country CHAR(2) NOT NULL DEFAULT 'GB',
  CONSTRAINT addresses_customer_fk FOREIGN KEY (customer_id) REFERENCES customers (id) ON DELETE CASCADE
);

CREATE TABLE products (
  id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
  sku VARCHAR(32) NOT NULL UNIQUE,
  name VARCHAR(255) NOT NULL,
  price DECIMAL(10,2) NOT NULL,
  active TINYINT(1) NOT NULL DEFAULT 1
);

CREATE TABLE inventory (
  product_id BIGINT UNSIGNED PRIMARY KEY,
  on_hand INT NOT NULL DEFAULT 0,
  reserved INT NOT NULL DEFAULT 0,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
  CONSTRAINT inventory_product_fk FOREIGN KEY (product_id) REFERENCES products (id)
);

CREATE TABLE carts (
  id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
  customer_id BIGINT UNSIGNED,
  items JSON NOT NULL,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT carts_customer_fk FOREIGN KEY (customer_id) REFERENCES customers (id)
);

CREATE TABLE orders (
  id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
  customer_id BIGINT UNSIGNED NOT NULL,
  status ENUM('pending','paid','shipped','delivered','cancelled','refunded') NOT NULL DEFAULT 'pending',
  total DECIMAL(10,2) NOT NULL DEFAULT 0,
  currency CHAR(3) NOT NULL DEFAULT 'GBP' COMMENT 'ISO 4217',
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  shipped_at DATETIME NULL,
  notes TEXT NULL COMMENT 'Internal only',
  CONSTRAINT orders_customer_id_fkey FOREIGN KEY (customer_id) REFERENCES customers (id)
    ON DELETE RESTRICT ON UPDATE CASCADE,
  INDEX orders_status_created_idx (status, created_at DESC)
);

CREATE TABLE order_items (
  id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
  order_id BIGINT UNSIGNED NOT NULL,
  product_id BIGINT UNSIGNED NOT NULL,
  quantity INT NOT NULL,
  unit_price DECIMAL(10,2) NOT NULL,
  CONSTRAINT order_items_order_fk FOREIGN KEY (order_id) REFERENCES orders (id) ON DELETE CASCADE,
  CONSTRAINT order_items_product_fk FOREIGN KEY (product_id) REFERENCES products (id)
);

CREATE TABLE payments (
  id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
  order_id BIGINT UNSIGNED NOT NULL,
  method ENUM('card','paypal','bank_transfer','gift_card') NOT NULL,
  amount DECIMAL(10,2) NOT NULL,
  paid_at DATETIME NOT NULL,
  CONSTRAINT payments_order_fk FOREIGN KEY (order_id) REFERENCES orders (id)
);

CREATE TABLE refunds (
  id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
  payment_id BIGINT UNSIGNED NOT NULL,
  amount DECIMAL(10,2) NOT NULL,
  reason VARCHAR(255),
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT refunds_payment_fk FOREIGN KEY (payment_id) REFERENCES payments (id)
);

CREATE TABLE reviews (
  id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
  product_id BIGINT UNSIGNED NOT NULL,
  customer_id BIGINT UNSIGNED NOT NULL,
  rating TINYINT NOT NULL,
  body TEXT,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT reviews_product_fk FOREIGN KEY (product_id) REFERENCES products (id),
  CONSTRAINT reviews_customer_fk FOREIGN KEY (customer_id) REFERENCES customers (id)
);

CREATE TABLE shipments (
  id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
  order_id BIGINT UNSIGNED NOT NULL,
  carrier VARCHAR(32) NOT NULL,
  tracking_no VARCHAR(32),
  shipped_at DATETIME NOT NULL,
  delivered_at DATETIME,
  CONSTRAINT shipments_order_fk FOREIGN KEY (order_id) REFERENCES orders (id)
);

INSERT INTO users (email, role)
WITH RECURSIVE g(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM g WHERE n < 12)
SELECT CONCAT('staff', n, '@shop.example'), IF(n <= 2, 'admin', 'staff') FROM g;

INSERT INTO customers (email, name, country, vip, created_at)
WITH RECURSIVE g(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM g WHERE n < 500)
SELECT CONCAT('customer', n, '@example.com'), CONCAT('Customer ', n),
       ELT(1 + FLOOR(RAND(n) * 7), 'GB', 'GB', 'GB', 'IE', 'DE', 'FR', 'US'),
       RAND(n * 3) < 0.1, NOW() - INTERVAL FLOOR(RAND(n * 7) * 700) DAY
FROM g;

INSERT INTO addresses (customer_id, line1, city, postcode, country)
SELECT id, CONCAT(1 + id % 200, ' High Street'),
       ELT(1 + id % 7, 'London', 'Leeds', 'Bristol', 'Dublin', 'Berlin', 'Paris', 'Boston'),
       UPPER(LEFT(MD5(id), 3)), country
FROM customers;

INSERT INTO products (sku, name, price, active)
WITH RECURSIVE g(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM g WHERE n < 120)
SELECT CONCAT('SKU-', LPAD(n, 4, '0')), CONCAT('Product ', n), ROUND(5 + RAND(n) * 295, 2), RAND(n * 5) > 0.08
FROM g;

INSERT INTO inventory (product_id, on_hand, reserved)
SELECT id, FLOOR(RAND(id) * 400), FLOOR(RAND(id * 2) * 20) FROM products;

INSERT INTO carts (customer_id, items, updated_at)
WITH RECURSIVE g(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM g WHERE n < 80)
SELECT 1 + FLOOR(RAND(n) * 500), JSON_ARRAY(JSON_OBJECT('product_id', 1 + n % 120, 'qty', 1 + n % 3)),
       NOW() - INTERVAL FLOOR(RAND(n) * 72) HOUR
FROM g;

INSERT INTO orders (customer_id, status, total, currency, created_at, shipped_at, notes)
SELECT customer_id, status, total, currency, created_at,
       IF(status IN ('shipped', 'delivered'), created_at + INTERVAL FLOOR(RAND(n) * 30) HOUR, NULL),
       IF(RAND(n * 11) < 0.06, ELT(1 + n % 5, 'Leave with neighbour', 'Gift wrap', 'Duplicate order',
          'Damaged in transit', 'Call before delivery'), NULL)
FROM (
  WITH RECURSIVE g(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM g WHERE n < 1284)
  SELECT n, 1 + FLOOR(RAND(n) * 500) AS customer_id,
         ELT(1 + FLOOR(RAND(n * 3) * 11), 'pending', 'paid', 'paid', 'paid', 'shipped', 'shipped',
             'delivered', 'delivered', 'delivered', 'cancelled', 'refunded') AS status,
         ROUND(5 + RAND(n * 5) * 320, 2) AS total,
         ELT(1 + FLOOR(RAND(n * 7) * 6), 'GBP', 'GBP', 'GBP', 'GBP', 'EUR', 'USD') AS currency,
         NOW() - INTERVAL FLOOR(RAND(n * 13) * 86400 * 60) SECOND AS created_at
  FROM g
) o
ORDER BY created_at;

INSERT INTO order_items (order_id, product_id, quantity, unit_price)
SELECT o.id, p.id, 1 + o.id % 3, p.price FROM orders o JOIN products p ON p.id = 1 + o.id % 120;

INSERT INTO payments (order_id, method, amount, paid_at)
SELECT id, ELT(1 + id % 6, 'card', 'card', 'card', 'paypal', 'bank_transfer', 'gift_card'), total,
       created_at + INTERVAL 2 MINUTE
FROM orders WHERE status <> 'pending';

INSERT INTO refunds (payment_id, amount, reason, created_at)
SELECT p.id, p.amount, 'Customer request', p.paid_at + INTERVAL 3 DAY
FROM payments p JOIN orders o ON o.id = p.order_id WHERE o.status = 'refunded';

INSERT INTO reviews (product_id, customer_id, rating, body, created_at)
WITH RECURSIVE g(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM g WHERE n < 600)
SELECT 1 + n % 120, 1 + FLOOR(RAND(n) * 500), 1 + n % 5, CONCAT('Review text ', n),
       NOW() - INTERVAL FLOOR(RAND(n * 3) * 90) DAY
FROM g;

INSERT INTO shipments (order_id, carrier, tracking_no, shipped_at, delivered_at)
SELECT id, ELT(1 + id % 4, 'Royal Mail', 'DPD', 'DHL', 'UPS'), UPPER(LEFT(MD5(id), 12)), shipped_at,
       IF(status = 'delivered', shipped_at + INTERVAL 2 DAY, NULL)
FROM orders WHERE shipped_at IS NOT NULL;

CREATE VIEW orders_daily_summary AS
SELECT DATE(created_at) AS day, COUNT(*) AS orders, SUM(total) AS revenue
FROM orders WHERE status <> 'cancelled' GROUP BY DATE(created_at);

CREATE VIEW customer_lifetime_value AS
SELECT c.id, c.email, COUNT(o.id) AS orders, COALESCE(SUM(o.total), 0) AS revenue
FROM customers c LEFT JOIN orders o ON o.customer_id = c.id AND o.status <> 'cancelled'
GROUP BY c.id, c.email;

CREATE VIEW low_stock_products AS
SELECT p.id, p.sku, p.name, i.on_hand - i.reserved AS available
FROM products p JOIN inventory i ON i.product_id = p.id WHERE i.on_hand - i.reserved < 20;

ANALYZE TABLE orders, customers, products;
