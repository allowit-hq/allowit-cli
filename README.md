# Allowit CLI

The `allowit` command provides developer workflows, local key custody, portable drafts, wallet profiles and agent requests.

**Go on. On your terms.**

Install the command (a release binary when one exists, otherwise a source build; see the [CLI reference](cli/README.md#install)):

```sh
curl -fsSL https://app.allowit.ai/install | sh
```

Each Allowit app serves `/install` at its own origin, for the branch of this repository that the deployment links.
An agent starts with the line that its owner copied from the app: that install URL and `allowit POLICY_ID help`.

Build and test with Rust 1.98 or later:

```sh
cargo build --locked --release --bin allowit
cargo test --locked --workspace
cargo run --locked --bin allowit -- --help
```

`cargo test` runs the hermetic tests: no socket, no network. An end-to-end test binds a local
socket for a fixture server. It carries `#[cfg_attr(not(feature = "e2e"), ignore = ...)]`, so the
default run lists it as ignored. The `e2e` feature runs the whole suite, end-to-end tests included:

```sh
cargo test --locked --workspace --features e2e
```

`scripts/no-network.sh` runs a command with IP networking denied: on Linux in a systemd unit that
may create no IP socket (it uses `sudo`), on macOS in the built-in sandbox. CI runs the hermetic
tests through it, so a hermetic test that opens a socket, even on 127.0.0.1, fails:

```sh
cargo test --locked --workspace --no-run
scripts/no-network.sh cargo test --locked --offline --workspace
```

Mark a new test that needs a socket or the network with the `e2e` attribute.

See the [CLI reference](cli/README.md), [portable policy reference](docs/policy-reference.md) and [known issues](known_issues.md).
The CLI consumes the pinned [Allowit SDK](https://github.com/allowit-hq/allowit-sdk) for shared execution and owner workflows.
The CLI names no vendor. It reaches every rail through the SDK plugin registry.

## Architecture

The CLI owns arguments, terminal output and local keys. The SDK supplies reusable execution and owner workflows. Rail libraries supply native signing and settlement semantics. The server owns trusted enforcement. See the [SDK architecture](https://github.com/allowit-hq/allowit-sdk#architecture).
