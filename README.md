# TraceFold

TraceFold is a research artifact and local CLI for query-preserving compression of tiered telemetry archives. It retains exact recent records and all error records while replacing older successful payloads with exact aggregate views for a declared query contract.

> Status: implementation in progress. The specification is in [PRD.md](PRD.md).

## Build

```bash
cargo build --release --locked
cargo test --workspace --locked
```

The reporting package requires Python 3.11+ and `uv`; the research site uses Node 24 and pnpm. Downloaded corpora, normalized logs, archives, and temporary benchmark data are never committed.

## License

Licensed under either Apache-2.0 or MIT, at your option.

