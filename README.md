# Allowit CLI

The `allowit` command provides developer workflows, local key custody, portable drafts, wallet profiles and agent requests.

**Go on. On your terms.**

Build and test with Rust 1.98 or later:

```sh
cargo build --locked --release --bin allowit
cargo test --locked --workspace
cargo run --locked --bin allowit -- --help
```

See the [CLI reference](cli/README.md), [portable policy reference](docs/policy-reference.md) and [known issues](known_issues.md).
The CLI consumes the pinned [Allowit SDK](https://github.com/allowit-hq/allowit-sdk) for shared execution and owner workflows.
The CLI names no vendor. It reaches every rail through the SDK plugin registry.
`node scripts/vendor-boundary.mjs --self-test && node scripts/vendor-boundary.mjs` checks that boundary; `cargo test` runs it.

## Architecture

The CLI owns arguments, terminal output and local keys. The SDK supplies reusable execution and owner workflows. Rail libraries supply native signing and settlement semantics. The server owns trusted enforcement. See the [SDK architecture](https://github.com/allowit-hq/allowit-sdk#architecture).
