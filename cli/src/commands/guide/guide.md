# AllowIt agent guide

An AllowIt policy wallet holds funds. Its policy decides what the agent key may
spend. Your agent key signs requests. The server and rail enforce the installed
policy and native authority before settlement.

## Rails and this build

The CLI names no rail. The SDK registers each vendor plugin, and the CLI
reaches every rail through that registry with one generic adapter.
The SDK shares execution and recovery across CLI and server callers.
Vendor plugins own native encoding, signatures, simulation and settlement.
Core supplies the plugin contract, portable policy types and common transport
and storage primitives.

The [workspace manifest](https://github.com/allowit-hq/allowit-cli/blob/main/Cargo.toml)
and [lockfile](https://github.com/allowit-hq/allowit-cli/blob/main/Cargo.lock)
select the SDK and its exact source version.
The [SDK plugin registration](https://github.com/allowit-hq/allowit-sdk/blob/main/crates/allowit-sdk/src/plugins/registered.rs)
lists the rails in the executable.
These links describe the source checkout. Inspect your installed build with the commands below.

## 1. Inspect and select a rail

```sh
allowit version
allowit rail ls --json
allowit rail show __ALLOWIT_EXAMPLE_RAIL__ --json
allowit rail use __ALLOWIT_EXAMPLE_RAIL__
```

Rail examples use your selected rail, or the first rail registered in this build.
This output is the installed build's inventory, including canonical identifiers and accepted aliases.
`rail show` reports its operations, available commands, key curve and asset metadata.
A command that needs a rail exits 3 until one is selected.

The selected rail's vendor plugin defines its public wallet profile, address
formats and native asset syntax. Vendor-specific inputs belong to the plugin,
not the common command contract.
See [known issues](https://github.com/allowit-hq/allowit-cli/blob/main/known_issues.md) for profile and native-binding workarounds.
`ALLOWIT_URL` selects the server origin.

## 2. Create and select your key

```sh
allowit keys generate agent
allowit keys use agent
```

Private keys stay in the config directory. No flag accepts a secret.
Use `keys fund agent` only when `rail show` lists `keys fund` as available.

## 3. Use the owner's wallet

```sh
allowit wallet use WALLET
allowit wallet show
allowit policy show
```

Replace `WALLET` with the owner's public wallet address.
Complete the selected adapter's configuration before reading or using its profile.
Selecting an address does not import a profile or authorize spending.

## 4. Spend

```sh
allowit exec --op transfer --to DESTINATION --amount ASSET:5 --request-id pay-0001
allowit status pay-0001 --wait 60s
```

Replace `DESTINATION` and `ASSET:5` with values accepted by the selected adapter.
Use only operations reported by `rail show`.
The selected key must match the profile's agent before signing.
`exec` prints a generated request ID on stderr when none was supplied.
After exit 5, retry only with the same `--request-id`, or read the result with `status`.
`--wait` bounds status polling only; prepare and submit are not bounded.
A settled result shows its transaction, height, fee and explorer link, with each amount in the asset's unit.
When the result carries them, it also shows the charged fee and each spending limit: limit, spent and remaining.
`history` lists local requests.

## Generate an owner draft

An owner can generate a portable draft with a separate named local key:

```sh
allowit keys import owner owner.key
allowit --json policy generate --owner-key owner --origin http://localhost:5173 \
  --kind standard --new-wallet 'Pay only my approved recipient'
```

`--kind` is `native`, `simple`, `standard` or `custom`. `allowit policy kinds` explains each kind of the
selected rail, and `allowit policy kinds --server` lists the kinds that the server accepts.
The draft's `walletKind` names the kind that the server bound; the CLI refuses a draft of another kind.
The sign-in Origin must be allowed by that backend.
The owner key does not replace the selected agent key. `--new-wallet` requires an advertised template;
`--installation ID` explicitly selects an existing owner installation instead.
Generation returns a draft. It does not deploy, fund or activate a wallet.
Exit 11 means that owner input remains unresolved.
After uncertain generation, inspect the owner workspace before creating another draft.

## Exit codes

| Code | Meaning | What to do |
|---|---|---|
| 0 | ok | |
| 2 | usage error | fix the command |
| 3 | configuration, or not available on this rail | follow the hint |
| 4 | rejected by the contract (paused, wrong asset), or refused by the server with a code | ask the owner |
| 5 | uncertain | preserve the original operation; inspect an uncertain generation before creating another |
| 6 | replayed | the earlier result stands |
| 7 | network unavailable | check the network, then retry with the same request ID |
| 10 | owner signature needed | the owner opens the printed link |
| 11 | waiting for owner input | stop and report the prompt |
| 12 | pending | `allowit status ID --wait 60s` |
| 20 | denied by the policy | do not retry |

## Output

Text is the default. Add `--json` for JSON. Data goes to stdout, including an
owner signing link, which is the `signingUrl` field. Hints, request IDs and the
progress of long steps go to stderr; `-q` drops progress, and `--json` prints it
only with `-v`.
