-- Seed for shop_dev.public, matching design/screens. Deterministic: same data every run.
-- Re-runnable: drops and recreates the public schema.

DROP SCHEMA IF EXISTS public CASCADE;
CREATE SCHEMA public;
SELECT setseed(0.42);

CREATE TYPE order_status AS ENUM ('pending', 'paid', 'shipped', 'delivered', 'cancelled', 'refunded');
CREATE TYPE payment_method AS ENUM ('card', 'paypal', 'bank_transfer', 'gift_card');

CREATE TABLE users (
  id bigserial PRIMARY KEY,
  email text NOT NULL UNIQUE,
  role text NOT NULL DEFAULT 'staff',
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE customers (
  id bigserial PRIMARY KEY,
  email text NOT NULL UNIQUE,
  name text NOT NULL,
  country char(2) NOT NULL DEFAULT 'GB',
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE addresses (
  id bigserial PRIMARY KEY,
  customer_id bigint NOT NULL REFERENCES customers (id) ON DELETE CASCADE,
  line1 text NOT NULL,
  city text NOT NULL,
  postcode text NOT NULL,
  country char(2) NOT NULL DEFAULT 'GB'
);

CREATE TABLE products (
  id bigserial PRIMARY KEY,
  sku text NOT NULL UNIQUE,
  name text NOT NULL,
  price numeric(10,2) NOT NULL,
  active boolean NOT NULL DEFAULT true
);

CREATE TABLE inventory (
  product_id bigint PRIMARY KEY REFERENCES products (id),
  on_hand integer NOT NULL DEFAULT 0,
  reserved integer NOT NULL DEFAULT 0,
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE carts (
  id bigserial PRIMARY KEY,
  customer_id bigint REFERENCES customers (id),
  items jsonb NOT NULL DEFAULT '[]',
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE orders (
  id bigserial PRIMARY KEY,
  customer_id bigint NOT NULL,
  status order_status NOT NULL DEFAULT 'pending',
  total numeric(10,2) NOT NULL DEFAULT 0,
  currency char(3) NOT NULL DEFAULT 'GBP',
  created_at timestamptz NOT NULL DEFAULT now(),
  shipped_at timestamptz NULL,
  notes text NULL,
  CONSTRAINT orders_customer_id_fkey FOREIGN KEY (customer_id)
    REFERENCES customers (id) ON DELETE RESTRICT ON UPDATE CASCADE
);
COMMENT ON COLUMN orders.status IS 'enum: pending, paid, shipped, delivered, cancelled, refunded';
COMMENT ON COLUMN orders.currency IS 'ISO 4217';
COMMENT ON COLUMN orders.notes IS 'Internal only';
CREATE INDEX orders_customer_id_idx ON orders (customer_id);
CREATE INDEX orders_status_created_idx ON orders (status, created_at DESC);

CREATE TABLE order_items (
  id bigserial PRIMARY KEY,
  order_id bigint NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
  product_id bigint NOT NULL REFERENCES products (id),
  quantity integer NOT NULL CHECK (quantity > 0),
  unit_price numeric(10,2) NOT NULL
);

CREATE TABLE payments (
  id bigserial PRIMARY KEY,
  order_id bigint NOT NULL REFERENCES orders (id),
  method payment_method NOT NULL,
  amount numeric(10,2) NOT NULL,
  paid_at timestamptz NOT NULL
);

CREATE TABLE refunds (
  id bigserial PRIMARY KEY,
  payment_id bigint NOT NULL REFERENCES payments (id),
  amount numeric(10,2) NOT NULL,
  reason text,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE reviews (
  id bigserial PRIMARY KEY,
  product_id bigint NOT NULL REFERENCES products (id),
  customer_id bigint NOT NULL REFERENCES customers (id),
  rating smallint NOT NULL CHECK (rating BETWEEN 1 AND 5),
  body text,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE shipments (
  id bigserial PRIMARY KEY,
  order_id bigint NOT NULL REFERENCES orders (id),
  carrier text NOT NULL,
  tracking_no text,
  shipped_at timestamptz NOT NULL,
  delivered_at timestamptz
);

-- Data ---------------------------------------------------------------------------------------

INSERT INTO users (email, role)
SELECT 'staff' || g || '@shop.example', CASE WHEN g <= 2 THEN 'admin' ELSE 'staff' END
FROM generate_series(1, 12) g;

INSERT INTO customers (email, name, country, created_at)
SELECT 'customer' || g || '@example.com',
       'Customer ' || g,
       (ARRAY['GB', 'GB', 'GB', 'IE', 'DE', 'FR', 'US'])[1 + floor(random() * 7)::int],
       now() - (random() * interval '700 days')
FROM generate_series(1, 500) g;

-- Customers that appear by name in the mockups.
UPDATE customers c SET email = v.email, name = v.name
FROM (VALUES
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
) AS v (id, email, name)
WHERE c.id = v.id;

INSERT INTO addresses (customer_id, line1, city, postcode, country)
SELECT c.id, (1 + floor(random() * 200)::int) || ' High Street',
       (ARRAY['London', 'Leeds', 'Bristol', 'Dublin', 'Berlin', 'Paris', 'Boston'])[1 + floor(random() * 7)::int],
       upper(substr(md5(c.id::text), 1, 3)) || ' ' || floor(random() * 9)::int || 'AB',
       c.country
FROM customers c;

INSERT INTO products (sku, name, price, active)
SELECT 'SKU-' || lpad(g::text, 4, '0'), 'Product ' || g,
       round((5 + random() * 295)::numeric, 2), random() > 0.08
FROM generate_series(1, 120) g;

INSERT INTO inventory (product_id, on_hand, reserved)
SELECT id, floor(random() * 400)::int, floor(random() * 20)::int FROM products;

INSERT INTO carts (customer_id, items, updated_at)
SELECT 1 + floor(random() * 500)::int,
       jsonb_build_array(jsonb_build_object('product_id', 1 + floor(random() * 120)::int, 'qty', 1 + floor(random() * 3)::int)),
       now() - random() * interval '3 days'
FROM generate_series(1, 80);

-- 1,284 orders over the last 60 days, ordered by created_at so ids increase with time.
INSERT INTO orders (customer_id, status, total, currency, created_at, shipped_at, notes)
SELECT customer_id, status, total, currency, created_at,
       CASE WHEN status IN ('shipped', 'delivered') THEN created_at + random() * interval '30 hours' END,
       CASE WHEN random() < 0.06
            THEN (ARRAY['Leave with neighbour', 'Gift wrap', 'Duplicate order', 'Damaged in transit', 'Call before delivery'])[1 + floor(random() * 5)::int]
       END
FROM (
  SELECT 1 + floor(random() * 500)::int AS customer_id,
         (ARRAY['pending', 'paid', 'paid', 'paid', 'shipped', 'shipped', 'delivered', 'delivered', 'delivered', 'cancelled', 'refunded'])[1 + floor(random() * 11)::int]::order_status AS status,
         round((5 + random() * 320)::numeric, 2) AS total,
         (ARRAY['GBP', 'GBP', 'GBP', 'GBP', 'EUR', 'USD'])[1 + floor(random() * 6)::int] AS currency,
         date_trunc('second', now() - random() * interval '60 days') AS created_at
  FROM generate_series(1, 1284)
  ORDER BY created_at
) o;

INSERT INTO order_items (order_id, product_id, quantity, unit_price)
SELECT o.id, p.id, 1 + floor(random() * 3)::int, p.price
FROM orders o
CROSS JOIN LATERAL (
  SELECT id, price FROM products WHERE o.id > 0 ORDER BY random() LIMIT 1 + floor(random() * 3)::int
) p;

INSERT INTO payments (order_id, method, amount, paid_at)
SELECT id,
       (ARRAY['card', 'card', 'card', 'paypal', 'bank_transfer', 'gift_card'])[1 + floor(random() * 6)::int]::payment_method,
       total, created_at + interval '2 minutes'
FROM orders WHERE status <> 'pending';

INSERT INTO refunds (payment_id, amount, reason, created_at)
SELECT p.id, p.amount, 'Customer request', p.paid_at + interval '3 days'
FROM payments p JOIN orders o ON o.id = p.order_id
WHERE o.status = 'refunded';

INSERT INTO reviews (product_id, customer_id, rating, body, created_at)
SELECT 1 + floor(random() * 120)::int, 1 + floor(random() * 500)::int,
       1 + floor(random() * 5)::int, 'Review text ' || g, now() - random() * interval '90 days'
FROM generate_series(1, 600) g;

INSERT INTO shipments (order_id, carrier, tracking_no, shipped_at, delivered_at)
SELECT id, (ARRAY['Royal Mail', 'DPD', 'DHL', 'UPS'])[1 + floor(random() * 4)::int],
       upper(substr(md5(id::text), 1, 12)), shipped_at,
       CASE WHEN status = 'delivered' THEN shipped_at + interval '2 days' END
FROM orders WHERE shipped_at IS NOT NULL;

-- Views --------------------------------------------------------------------------------------

CREATE VIEW orders_daily_summary AS
SELECT date_trunc('day', created_at)::date AS day, count(*) AS orders, sum(total) AS revenue
FROM orders WHERE status <> 'cancelled'
GROUP BY 1;

CREATE VIEW customer_lifetime_value AS
SELECT c.id, c.email, count(o.id) AS orders, coalesce(sum(o.total), 0) AS revenue
FROM customers c LEFT JOIN orders o ON o.customer_id = c.id AND o.status <> 'cancelled'
GROUP BY c.id, c.email;

CREATE VIEW low_stock_products AS
SELECT p.id, p.sku, p.name, i.on_hand - i.reserved AS available
FROM products p JOIN inventory i ON i.product_id = p.id
WHERE i.on_hand - i.reserved < 20;

-- Functions ----------------------------------------------------------------------------------

CREATE FUNCTION order_total(p_order_id bigint) RETURNS numeric LANGUAGE sql STABLE AS $$
  SELECT coalesce(sum(quantity * unit_price), 0) FROM order_items WHERE order_id = p_order_id
$$;

CREATE FUNCTION customer_order_count(p_customer_id bigint) RETURNS bigint LANGUAGE sql STABLE AS $$
  SELECT count(*) FROM orders WHERE customer_id = p_customer_id
$$;

CREATE FUNCTION available_stock(p_product_id bigint) RETURNS integer LANGUAGE sql STABLE AS $$
  SELECT on_hand - reserved FROM inventory WHERE product_id = p_product_id
$$;

CREATE FUNCTION mark_shipped(p_order_id bigint) RETURNS void LANGUAGE sql AS $$
  UPDATE orders SET status = 'shipped', shipped_at = now() WHERE id = p_order_id
$$;

CREATE FUNCTION refund_order(p_order_id bigint) RETURNS void LANGUAGE sql AS $$
  UPDATE orders SET status = 'refunded' WHERE id = p_order_id
$$;

CREATE FUNCTION touch_updated_at() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  NEW.updated_at := now();
  RETURN NEW;
END
$$;

CREATE FUNCTION revenue_between(p_from timestamptz, p_to timestamptz) RETURNS numeric LANGUAGE sql STABLE AS $$
  SELECT coalesce(sum(total), 0) FROM orders
  WHERE created_at >= p_from AND created_at < p_to AND status <> 'cancelled'
$$;

CREATE TRIGGER inventory_touch BEFORE UPDATE ON inventory
  FOR EACH ROW EXECUTE FUNCTION touch_updated_at();
CREATE TRIGGER carts_touch BEFORE UPDATE ON carts
  FOR EACH ROW EXECUTE FUNCTION touch_updated_at();

ANALYZE;
