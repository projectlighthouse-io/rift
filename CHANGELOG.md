# Changelog

Notable changes to Rift. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- New typefaces: Inter for the interface and data (numbers line up in the grid), JetBrains Mono
  only for SQL, DDL and JSON. Both are bundled with the app.
- Redesigned to the updated mockups: zinc palette, a breadcrumb top bar (connection, database,
  schema) instead of connection tabs, content in a floating white panel, pill tabs, and roomier
  40px grid rows with no vertical lines. Edited and focused cells show their value in a soft pill.
- Switch connections from the connection crumb or ⌘⇧C; right-click an open connection to
  disconnect it. The schema picker moved from the sidebar to the schema crumb.
- Connections are identified by a neutral engine tile (Pg, My, Sq, Ch, Rd, Fd) instead of a
  colour tag; the tag picker is gone from the connection dialog.
- ⌘E still edits the highlighted saved connection; the Edit button is gone, as in the design.

### Added

- Connection errors are selectable, with a button to copy them.

### Fixed

- PostgreSQL and MySQL can now use TLS. sqlx was built without TLS support, so servers that
  require encryption (DigitalOcean, most managed databases) refused every connection.
- PostgreSQL connects time out after 10 s again instead of waiting on the OS TCP timeout.
- Scrolling the data grid vertically no longer drifts it sideways, and a sideways swipe no
  longer scrolls rows.

### Performance

- PostgreSQL makes one round trip per query, table page, describe and sidebar load, down from
  5 to 13. Against a server ~200 ms away: a query takes ~205 ms (was ~1,030 ms) and describing a
  table ~210 ms (was ~2,630 ms).
- The sidebar and table views load their data concurrently instead of one call after another.
- Dev builds optimize dependencies, so the UI is usable unoptimized (scrolling was laggy).

## [0.1.0]

- First release.

[Unreleased]: https://github.com/projectlighthouse-io/rift/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/projectlighthouse-io/rift/releases/tag/v0.1.0
