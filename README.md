# Allowit CLI

The `allowit` command provides developer workflows, local key custody, portable drafts, wallet profiles and agent requests.

**Go on. On your terms.**

Install the command (a release binary when one exists, otherwise a source build; see the [CLI reference](cli/README.md#install)):

```sh
curl -fsSL https://raw.githubusercontent.com/allowit-hq/allowit-cli/main/install.sh | sh
```

An agent starts with the line that its owner copied from the app: `allowit POLICY_ID help`.

Build and test with Rust 1.98 or later:

```sh
cargo build --locked --release --bin allowit
cargo test --locked --workspace
cargo run --locked --bin allowit -- --help
```

See the [CLI reference](cli/README.md), [portable policy reference](docs/policy-reference.md) and [known issues](known_issues.md).
The CLI consumes the pinned [Allowit SDK](https://github.com/allowit-hq/allowit-sdk) for shared execution and owner workflows.
The CLI names no vendor. It reaches every rail through the SDK plugin registry.

## Architecture

The CLI owns arguments, terminal output and local keys. The SDK supplies reusable execution and owner workflows. Rail libraries supply native signing and settlement semantics. The server owns trusted enforcement. See the [SDK architecture](https://github.com/allowit-hq/allowit-sdk#architecture).
