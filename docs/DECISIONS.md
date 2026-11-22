# Decisions

Format: date, decision, reason.

## 2026-11-20 — M0

- **Workspace members listed explicitly.** `crates/*` would also match `crates/drivers`
  (no manifest); `crates/drivers/*` is globbed.
- **Edition 2024, resolver 3, MSRV 1.90.** Matches the local toolchain; GPUI tracks recent
  stable.
- **Package metadata inherited from `[workspace.package]`,** including `publish = false`.
- **`registry` enables every driver feature by default.** Release builds ship all drivers;
  tests and slim builds use `--no-default-features --features ...`.
- **`clippy.toml` allows `unwrap`/`expect` in tests** (`allow-unwrap-in-tests`), so test
  modules need no per-file `allow`.
- **Dependency check uses `cargo tree --prune registry`.** Anything with a `-driver` name left
  in `app`'s tree after pruning `registry` is a violation. The script also checks the direct
  workspace deps of `types`, `traits`, `dialect` and every driver.
- **Dev tasks via `Makefile`** (`make dev-up`, `dev-seed`, `dev-down`); `make` is everywhere.
- **Redis seed runs inside the container** (`redis-cli --pipe`), so no local `redis-cli` is
  needed.
- **Third-party dependency versions are pinned when first used** (M1 onward) and recorded
  here; M0 stubs have none.
- **Host ports overridable** via `RIFT_PG_PORT` / `RIFT_REDIS_PORT` (defaults 5432 / 6379) for
  machines where another Postgres or Redis already holds the default port.

## 2026-11-20 — M1

- **Futures are `futures::future::BoxFuture`,** not `async fn` in traits: capability traits
  must be object safe (`&dyn Browse`).
- **`Query::execute` runs one statement.** Splitting scripts into statements happens in
  `dialect` (via `sqlparser`) so every driver gets the same behaviour.
- **Rows stream as `BoxStream<Result<Vec<Row>>>`** of up to `PAGE_SIZE` (500) rows; drivers
  feed it from a bounded channel of 4 pages.
- **`Transactions::atomic(&[TxOp])`** covers both families: SQL statements or key writes.
  Unsupported ops fail before anything is written.
- **Stores are synchronous.** They are local SQLite; the app calls them off the UI thread.
- **Conformance fixture:** a 3-row table `(id, name, score)` the driver harness creates. SQL
  suites restore it after mutating, so all suites share one table.
- **`registry` feature `fake`** adds `testkit`'s `FakeSqlFactory` and `FakeKvFactory`; CI runs
  `cargo test -p registry --features fake`.

## 2026-11-20 — M2

- **Rust 1.98 pinned** in `rust-toolchain.toml`: `gpui-pre-macos` 0.3.12 uses
  `Option::map_or_default`, stable only from 1.97/1.98.
- **GPUI:** `gpui-component =0.10.0`, which requires `gpui = { package = "gpui-pre",
  version = "=0.3.12" }` and `gpui_platform = { package = "gpui-pre-platform",
  version = "=0.3.12" }` (`Application::with_platform(gpui_platform::current_platform(false))`).
  `runtime_shaders` avoids needing the Metal compiler from full Xcode.
- **sqlx 0.9 for Postgres (and later MySQL) instead of tokio-postgres / mysql_async**
  (owner's call). sqlx stays private to the driver crates; the app only sees the traits.
  - Results use `raw_sql` (simple protocol, text format), so every type is readable; unknown
    types become `Value::Unknown` with the server's text.
  - Result columns come from `Executor::prepare` before running the statement.
  - sqlx has no cancel API: the driver records `pg_backend_pid()` and runs
    `pg_cancel_backend(pid)` from a side connection when the token fires.
  - SQLite keeps `rusqlite` so long statements can be interrupted.
- **Values are inlined as SQL literals** by `dialect` (filters, edits) rather than bound as
  parameters: untyped string literals coerce to enums, numerics and timestamps in every engine,
  and the generated SQL is what the status bar and snapshots show. Escaping is per dialect.
- **Driver integration tests skip when Docker is unreachable,** unless `RIFT_REQUIRE_DOCKER=1`.
  Ports come from `RIFT_PG_PORT` etc.
- **Custom grid on GPUI `uniform_list`** instead of `gpui-component`'s `DataTable`: the design
  needs per-cell edit states, a row-number column, header name + type + key icons and a focus
  ring; owning ~300 lines is simpler than restyling the component. 100k rows scroll with no
  frame over 16 ms in release (`RIFT_DEV_AUTOSCROLL=1 RIFT_FRAME_LOG=1`, see below).
- **Icons are the mockups' own SVGs,** extracted from `design/html` into `crates/app/assets/icons`
  and served under `rift/`. Other paths fall back to `gpui-kit-assets`.
- **Source Serif 4 is bundled as static TTFs** (Regular, Italic, Semibold, Bold, OFL). Adobe ships
  no static Medium (500), so 500 renders with the nearest weight.
- **Mono font:** first installed of SF Mono, Menlo, JetBrains Mono, Consolas, DejaVu Sans Mono.
- **Window keeps the native title bar;** the 1440×900 design is the content area below it.
- **Dev switches:** `RIFT_DEV_QUERY` opens a result tab with that SQL at startup;
  `RIFT_DEV_AUTOSCROLL=1` scrolls every row one step per frame; `RIFT_FRAME_LOG=1` logs slow
  frames in release builds (debug builds always do).

## 2026-11-20 — M3

- **rusqlite 0.40** (not 0.43): `refinery` 0.10 supports rusqlite up to 0.40. The SQLite driver
  will use the same version so only one `libsqlite3-sys` is linked.
- **App records are JSON bodies** in SQLite with a few indexed columns (`store`), so new record
  fields need no migration. Passwords go only to the OS keychain (`keyring` v1 API, service
  `io.projectlighthouse.rift`, account = connection id).
- **Connections keep creation order;** the start screen groups them Local, Staging, Production,
  then other groups alphabetically.
- **Connection form layout is generic:** driver fields two per row, a `Host` followed by a `Port`
  share a row with a 96px port, `Group` fills the last slot, `advanced` fields go under
  "More options" with TLS, safe mode and SSH.
- **Shortcuts use `cmd-` on macOS and `ctrl-` elsewhere** (`cmd!` macro); this GPUI has no
  `secondary-` modifier.
- **Delete lives in the Edit dialog** (⌘E, then Delete) rather than a separate shortcut.
- **Inputs:** `ui::input(state, px)` sizes gpui-component's `Input` to the design's text size
  (it renders text at 0.875 × its size) and the wrapper adds 2px so text sits 10px in.
- **UI checks are driven by real input** (CGEvent clicks and key codes) against the running app;
  helpers live in the untracked `.tinker/`.

## 2026-11-20 — M4

- **Edits are inlined SQL in one transaction** (`dialect::edits`, snapshot-tested with `insta`).
  Updates and deletes must hit exactly one row by primary key, otherwise the whole save rolls
  back with "a row was changed or deleted by someone else".
- **Typed cell text becomes `Value::Text`** (empty or `NULL` means `NULL`) and is sent as an
  untyped literal, so the database coerces it to the column type and reports bad input itself.
- **New rows show column defaults** from `Describe` (casts stripped, string quotes removed);
  serial keys stay blank. Only cells the user sets are inserted, so defaults apply.
- **Inline editing focuses its input one tick later** (`defer_in`): the table view tracks focus
  for ⌘S and would otherwise take focus back on the same mouse-down.
- **FK links open a new table tab** with an `is` filter on the referenced column.
- **Dev switch `RIFT_DEV_INSECURE_SECRETS=1`** keeps passwords in a plaintext JSON file in the
  data dir. Unsigned rebuilds otherwise trigger a keychain password prompt on every launch,
  which blocks automated UI checks. Never the default.

## 2026-11-20 — M5

- **Identifiers are quoted only when needed** (not `[a-z_][a-z0-9_]*`, or a keyword), so DDL and
  edit SQL read like the design (`ALTER TABLE public.orders ADD COLUMN gift_message text`).
- **Structure changes are staged** and applied together with ⌘S; on Postgres they run in one
  transaction. Truncate is immediate after a second click ("Delete all rows?").
- **Drops are per-row trash buttons**; staged drops show red and struck through, staged adds
  green with a `NEW` badge.
- **"drag to reorder" is not shown:** Postgres and MySQL cannot reorder columns without
  rebuilding the table, so the hint would promise something Rift does not do.
- **sqlx statement cache disabled** for Postgres: after `ALTER TABLE`, cached `SELECT *`
  metadata returned stale columns (caught by the EditSchema conformance suite).

## 2026-11-20 — M6

- **Editor is gpui-component's `EditorState`** with Rift's own `InputHighlighter`
  (`sql_syntax.rs`): a small SQL tokenizer coloured with the design's palette (keywords
  `#7c3aed`, functions `#0369a1`, strings `#15803d`, numbers `#b45309`, comments faint
  italic). No tree-sitter theme to fight.
- **Current statement** is a `RangeDecoration` fill (`#f6f4f1`) from `dialect::split`. It covers
  the statement's text runs rather than full-width lines (the editor's decoration model).
  The decoration is only replaced when the range changes: replacing it notifies the editor,
  which re-runs the observer (an infinite loop otherwise).
- **Statement splitting lives in `dialect::split`** (quotes, `$tag$` strings, `--` and `/* */`
  comments, leading comments trimmed) so every driver sees the same statements.
- **Run all stops at the first failing statement**; each row-returning statement gets a result
  tab, everything else is logged in Messages. Every statement is written to `HistoryStore`.
- **Autocomplete** uses `CompletionProvider` fed with tables, views, functions and the columns
  of up to 200 objects in the default schema, plus SQL keywords.
- **Copy as CSV / JSON / INSERT** and **Export** (CSV via a save dialog) use `export.rs`.
- **Decimal and float cells group thousands** (`1,842.50`), as in the design; integers do not
  (ids stay `1041`).

## 2026-11-20 — M7

- **SQLite opens the file per operation** on a blocking thread (`spawn_blocking`): long reads
  never hold a lock other views need. Rows stream through a bounded channel from the blocking
  thread (`block_on(send)` applies backpressure). Cancel calls `sqlite3_interrupt` via the
  connection's interrupt handle. `BOOLEAN`-declared integers become `Value::Bool`.
- **MySQL on sqlx** like Postgres: text protocol results, `KILL QUERY <CONNECTION_ID()>` from a
  side connection to cancel, `found_rows` on so updates report matched rows, statement cache
  off. `tinyint(1)` (sqlx reports `BOOLEAN`) → bool, `UNSIGNED` → `UInt`, inline `ENUM` → text.
- **MySQL has no schema level:** the driver reports databases as schemas; the sidebar shows the
  database once when it equals the schema (data-driven, not a driver check).
- **Non-transactional DDL** (`EditSchema::transactional_ddl() == false`) shows a "Cannot be
  rolled back" badge next to pending structure changes.
- **The sidebar's database · schema box is a picker** of `Browse::schemas()`.
- **Docker MySQL is 8.4 LTS** (`RIFT_MYSQL_PORT`, root password `mysql`, database `shop`).

## 2026-11-20 — M8

- **redis 1.8 (latest stable) with a tokio `ConnectionManager`** (auto-reconnect). The 2.0 line
  is still a release candidate.
- **Key tree is filled by a background `SCAN` loop** (`COUNT 1000`, `MATCH` from the pattern box,
  pipelined `TYPE` per page) streaming batches to the UI; duplicates `SCAN` may return are
  dropped. Keys group by the text up to the first `:`; expanded groups show 4 keys, then
  "… N more" adds 200 at a time. Never `KEYS`.
- **Values are read and written whole** (`KeyValue::get/set`): `set` runs `DEL` + the typed
  write in `MULTI`/`EXEC` and restores the `PTTL`. Hash fields, list items, set and zset
  members are edited by writing the new whole value. Simple and correct; very large
  collections should get per-field ops later.
- **Memory panel** uses `INFO memory` and `DBSIZE` for totals and a 2,000-key `MEMORY USAGE`
  sample scaled to the keyspace for the breakdowns (marked "sampled").
- **The UI picks screens by capability:** `key_value()` swaps the SQL sidebar for the key tree,
  `memory_stats()` adds the right panel, `key_expiry()` shows TTL and Set TTL, and the
  "+" query tab and top-bar SQL button only appear when `query()` is available.
- **Conformance runs in Redis db 15** so the seed data in db 0 is untouched.

## 2026-11-20 — M9

- **ClickHouse over HTTP (reqwest), not the native protocol.** Results stream as
  `JSONCompactEachRowWithNamesAndTypes`, so any column type maps to a `Value` at runtime
  (unknown types like `AggregateFunction` fall back to `Value::Unknown`). Each query carries a
  `query_id`; cancel sends `KILL QUERY … ASYNC`. Read-only safe mode sets `readonly=2`.
- **ClickHouse is read-mostly:** no `EditRows`, no `Transactions`. It does `EditSchema`
  (non-transactional) and `TableTtl`. The sidebar says so whenever `edit_rows()` is `None`.
- **Charts aggregate in the database.** `dialect::chart::aggregate_sql` wraps the statement as a
  subquery (or uses the table) with one `GROUP BY`, date buckets per dialect, and
  `LIMIT MAX_CHART_POINTS + 1` to detect truncation. The 4M-row daily aggregation runs in
  ~70 ms on ClickHouse.
- **Charts are painted directly** with GPUI paths and quads (no chart library). Export PNG
  renders the same chart as SVG through resvg with the bundled serif font.
- **Pin to dashboard** appends a `ChartCard` to the connection's first dashboard, creating
  "Overview" if none exists (dashboards UI is M10).
- **Column stats** come from two queries (totals, then top 8 values) on the whole table, shown
  in a popover from the column header. "Chart this column" opens a query tab with the
  `GROUP BY`.
- **New conformance suite: `table_ttl_suite`** (set, read, remove a table TTL).

## 2026-11-21 — M10

- **Dashboards are document tabs** listed under DASHBOARDS in the SQL sidebar. Cards render
  with the M9 chart code plus two new kinds: `Number` (latest value, change vs the previous
  point, bare sparkline) and `BarList` (values sorted by count).
- **Each card refreshes on its own timer:** a 1 s ticker refreshes cards whose interval has
  elapsed since their last refresh. Interval, alert and removal are set from the card menu
  and saved through `DashboardStore`.
- **Alerts compare the latest value** with the rule and notify once when the rule starts
  firing (badge turns red), through the OS notifier (`osascript` on macOS, `notify-send`
  elsewhere).
- **The time range trims points after aggregating** (dates outside the range are dropped).
  The database still groups all time; a `WHERE` in `AggregateRequest` is the upgrade if
  dashboards get slow.
- **"Add chart" opens a query tab;** charts are built in a result's Chart tab and pinned.
- **CI test job drops debug info and frees runner disk;** the GPUI debug build plus four
  database services ran out of space.

## 2026-11-21 — M11

- **`libfdb_c` is loaded at runtime** (`libloading`), not linked, so Rift builds and runs
  without it; connecting explains how to install it (`RIFT_FDB_LIB` overrides the path).
  API version 710 works with every 7.x client library.
- **`fdb-ffi` is the only crate with `unsafe`.** It copies the workspace lints but sets
  `unsafe_code = "deny"` (not `forbid`) so `ffi.rs` alone can allow it; every block has a
  SAFETY comment. The driver itself keeps `forbid`.
- **Every operation is one retried transaction on a blocking thread** (5 s timeout), using
  `fdb_future_block_until_ready`. Scans are snapshot reads and report their read version.
  Values are bytes only (`KvValue::String`); pattern scans are unsupported.
- **Two new capabilities:** `KeyDirectories` (directory-layer listing and key counts) and
  `KeyEncoding` (tuple layer decode/encode). The UI picks the range browser when
  `key_directories()` exists and offers "Decode as Tuple" when `key_encoding()` exists.
- **Directory counts scan the range** (10k keys per transaction, capped at 1,000,000).
- **Tests and seed run in a Linux container** with the client library (`make fdb-test`,
  `make dev-seed`, `dev/fdb/run.sh`), sharing the FoundationDB container's network so
  `127.0.0.1:4500` works; CI installs the client `.deb`.

## 2026-11-21 — M12

- **The palette searches a snapshot** taken when it opens: tables and views of every open
  session, their dashboards, the last 200 history entries of open connections, and "Connect to"
  for saved connections that are not open. Functions and enums are left out (they do not open
  as tables).
- **Matching is a case-insensitive substring;** names that start with the query rank first,
  then shorter names. 10,000 objects search in well under a frame (unit test); at most 50
  results render.
- **⌘K is a global action on the workspace root,** which takes focus back when the
  connections screen (and its focused search box) goes away, so the shortcut keeps working.
- **⌘↵ opens a table in a new tab** even when one is already open.

## 2026-11-22 — M13

- **Read-only is a wrapper in `conn`, not a driver flag alone.** `ReadOnlyConnection` hides
  write-only capabilities (`EditRows`, `EditSchema`, `Transactions`), keeps reads of mixed ones
  and refuses their writes (key writes, TTL changes, restores), and checks SQL with
  `dialect::safety` before it reaches the server. Drivers that can also enforce read-only in
  the database still do (Postgres `default_transaction_read_only`, MySQL `READ ONLY`, SQLite
  read-only open, ClickHouse `readonly=2`).
- **Only plain reads pass the SQL check:** queries without `INTO`, `EXPLAIN` of a read, `SHOW`,
  `DESCRIBE`, `USE`. `SET` counts as a write (it could switch the server's read-only mode
  off), `EXPLAIN ANALYZE` of a write counts as a write (it runs it), and anything sqlparser
  cannot parse is blocked.
- **Confirm writes and unbounded statements inline,** with a warning bar above the results
  (or the table toolbar) instead of a modal: `UPDATE`/`DELETE` without `WHERE` always asks;
  "Confirm writes" connections ask before any writing statement and before saving edits.
- **Backup shells out where the engine has a standard tool** (`pg_dump`/`psql`,
  `mysqldump`/`mysql`) and uses SQLite's online backup API. Tools are found on `PATH` and
  the usual Homebrew/Postgres.app locations; a missing tool is `MissingDependency` with
  install instructions. Postgres dumps use `--clean --if-exists`; restore runs in one
  transaction and skips `SET transaction_timeout` lines newer `pg_dump` writes for older
  servers. Read-only connections can back up but not restore.
- **Backup and restore live in the command palette** ("Back up X…", "Restore X from
  backup…"); restore always asks first.
- **Table export pages through the whole table** (5,000 rows at a time, applied filters kept)
  to CSV, JSON or SQL `INSERT`s in the connection's dialect. **CSV import** maps columns by
  name (editable), previews 20 rows, and inserts through `EditRows` in batches of 500.
- **SSH tunnels open in `conn`** (russh): the driver sees `127.0.0.1:<local port>`. Key
  authentication only (configured key, else `~/.ssh/id_ed25519`/`id_rsa`). Host keys are
  trusted on first use and recorded in `~/.ssh/known_hosts`; a changed key is refused.
- **TLS** was already per driver (Disable / Prefer / Require).

## 2026-11-22 — M14

- **The app package is named `rift`** (it was `app`), so release artifacts, installers and the
  Homebrew formula carry the product name. The crate folder stays `crates/app`.
- **Releases come from `dist` (cargo-dist 0.34)**, configured in `dist-workspace.toml`:
  macOS arm64 and x86_64, Linux x86_64 and aarch64 (native ARM runner), Windows x64; shell,
  PowerShell and Homebrew installers. Pushing a `v*` tag runs `.github/workflows/release.yml`.
  The Homebrew formula ships as a release artifact; publishing it to
  `projectlighthouse-io/homebrew-tap` is off until that repository and a `HOMEBREW_TAP_TOKEN`
  secret exist. macOS ships two per-architecture archives rather than one
  universal binary; Homebrew and the installer pick the right one.
- **Release builds use the `dist` profile** (release + thin LTO).
- **The icon is an SVG** (`crates/app/assets/icon/rift.svg`): the coral database with a
  rift through it. `scripts/make-icon.sh` renders the PNGs (resvg, in an ignored test) and
  the `.icns` (iconutil).
- **About window** from the Rift menu: icon, version, "Rift — a Project Lighthouse project".
  ⌘Q quits.
