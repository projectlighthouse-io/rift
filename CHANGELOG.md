# Changelog

Notable changes to Rift. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Edit button on the selected saved connection (also ⌘E, now shown in the hint line).
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
