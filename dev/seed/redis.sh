#!/bin/sh
# Seed Redis with 48,211 keys matching design/screens/09-redis-keys.png.
# Runs inside the redis container: `docker compose exec -T redis sh /seed/redis.sh`.
# Re-runnable: flushes db 0 first.
set -eu

redis-cli FLUSHDB > /dev/null

# Emit RESP so `redis-cli --pipe` can bulk load without a round trip per key.
awk 'BEGIN {
  srand(42)
  # cart:<id>  hash with TTL (12,000)
  for (i = 1; i <= 12000; i++) {
    k = "cart:" i
    cmd("HSET", k, "items", int(rand() * 6) + 1, "total", sprintf("%.2f", rand() * 300), "currency", "GBP")
    cmd("EXPIRE", k, 600 + int(rand() * 3000))
  }
  # session:<token>  string with TTL (20,000)
  for (i = 1; i <= 20000; i++) {
    cmd("SET", sprintf("session:%08x", i * 2654435761 % 4294967296),
        "{\"user_id\":" int(rand() * 8000 + 1) ",\"ip\":\"10.0." int(rand() * 255) "." int(rand() * 255) "\"}",
        "EX", 900 + int(rand() * 6300))
  }
  # rate:<route>:<user>  counter with short TTL (8,000)
  for (i = 1; i <= 8000; i++) {
    cmd("SET", "rate:api:" i, int(rand() * 120), "EX", 3600 + int(rand() * 3600))
  }
  # user:<id>  hash, some with a set of roles (8,000 total)
  for (i = 1; i <= 7000; i++) {
    cmd("HSET", "user:" i, "email", "user" i "@example.com", "name", "User " i, "plan", (rand() < 0.2 ? "pro" : "free"))
  }
  for (i = 1; i <= 1000; i++) {
    cmd("SADD", "user:" i ":roles", "customer", (i % 50 == 0 ? "admin" : "reader"))
  }
  # leaderboard:<board>  zset (11)
  n = split("daily weekly monthly alltime uk ie de fr us referrals reviews", boards, " ")
  for (b = 1; b <= n; b++) {
    for (j = 1; j <= 500; j++) cmd("ZADD", "leaderboard:" boards[b], int(rand() * 100000), "user:" j)
  }
  # queue:<name>  stream (200)
  for (q = 1; q <= 200; q++) {
    for (j = 1; j <= 5; j++) cmd("XADD", "queue:jobs:" q, "*", "job", "send_email", "attempt", j)
  }
}
function cmd(a, b, c, d, e, f, g, h,    args, n, i, out) {
  n = split(a SUBSEP b SUBSEP c SUBSEP d SUBSEP e SUBSEP f SUBSEP g SUBSEP h, args, SUBSEP)
  while (n > 0 && args[n] == "") n--
  out = "*" n "\r\n"
  for (i = 1; i <= n; i++) out = out "$" length(args[i]) "\r\n" args[i] "\r\n"
  printf "%s", out
}' | redis-cli --pipe > /dev/null

echo "redis: $(redis-cli DBSIZE) keys"
