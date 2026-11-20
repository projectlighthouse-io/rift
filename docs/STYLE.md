# Rift style guide

Base: the [Google Rust style guide](https://google.github.io/styleguide/rustguide.html).
Mechanical rules live in `rustfmt.toml`, `clippy.toml` and `[workspace.lints]`; build on stable,
format with `cargo +nightly fmt --all`. The rules below are checked in review.

## Crates and modules

- Small crates. One file per abstraction.
- Non-leaf modules contain only `mod` declarations and re-exports.
- Layering (enforced by `scripts/check-deps.sh`):
  - `types` depends on no workspace crate.
  - `traits` depends only on `types` and does not re-export it.
  - Driver crates depend only on `types`, `traits`, `dialect` and their database client.
    Everything is private except `pub fn factory() -> Box<dyn DriverFactory>`.
  - `registry` is the only crate that names drivers; each sits behind a feature.
  - `app` never depends on a driver crate.

## Imports

- No glob imports (`use foo::*`), including enum variants.
- Import types unqualified (`use types::Value;`). Qualify free functions by their module
  (`registry::all()`, not a bare `all()`).

## Types

- Newtypes over aliases (`struct TableName(String)`, never `type TableName = String`).
- Named fields over tuples for anything with more than one meaning.
- Public enums are `#[non_exhaustive]`. Error types use `thiserror` in libraries; `anyhow`
  only inside `app`. Mark pure constructors and getters `#[must_use]`.
- `Arc` only where shared ownership is genuinely needed. Documented exception: connections
  (`Arc<dyn Connection>`) are shared between views and tasks.
- Prefer generics over `dyn`. Documented exception: capability traits are `dyn` so the UI can
  ask a connection at runtime what it supports.

## Errors and panics

- No `unwrap()` outside tests. Use `expect("why this cannot fail")` when an invariant holds.
- `panic!` only for violated invariants, never for bad input or I/O.

## Docs

- `///` on every public item. Each capability trait lists the databases that implement it.
- Record non-obvious decisions in `docs/DECISIONS.md`.

## Async

- Never await a driver on GPUI's executor. Database work runs on the Tokio runtime via
  `Spawner`; results come back through `spawn_db`.
- Blocking libraries go through `spawn_blocking`.
- Every query takes a `CancellationToken`.
