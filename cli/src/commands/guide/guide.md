# AllowIt agent guide

An AllowIt policy wallet holds funds. Its policy decides what the agent key may
spend. Your agent key signs requests. The server and rail enforce the installed
policy and native authority before settlement.

## Vendors and this build

A vendor adapter connects the common commands to a rail's native protocol.
The SDK engine shares execution and recovery across CLI and server callers.
Rail libraries own native encoding, signatures, simulation and settlement.
Core supplies portable policy types and common transport and storage primitives.

The [workspace manifest](https://github.com/allowit-hq/allowit-sdk/blob/main/Cargo.toml)
and [lockfile](https://github.com/allowit-hq/allowit-sdk/blob/main/Cargo.lock)
select vendor dependencies and their exact source versions.
The [rail table](https://github.com/allowit-hq/allowit-sdk/blob/main/engine/src/routing.rs)
and [adapter wiring](https://github.com/allowit-hq/allowit-sdk/blob/main/cli/src/vendors/mod.rs)
register them in the executable. Adding a Cargo dependency alone does not register an adapter.
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

Use the selected [adapter's configuration](https://github.com/allowit-hq/allowit-sdk/tree/main/cli/src/vendors)
for its public wallet profile, address formats and native asset syntax.
Vendor-specific profile inputs belong to adapter configuration, not the common command contract.
See [known issues](https://github.com/allowit-hq/allowit-sdk/blob/main/known_issues.md) for profile and native-binding workarounds.
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
`history` lists local requests.

## Generate an owner draft

An owner can generate a portable draft with a separate named local key:

```sh
allowit keys import owner owner.key
allowit --json policy generate --owner-key owner --origin http://localhost:5173 \
  --profile PROFILE --new-wallet 'Pay only my approved recipient'
```

Use the exact wallet `PROFILE` advertised by the owner's server for the selected adapter.
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
| 4 | rejected by the contract (paused, wrong asset) | ask the owner |
| 5 | uncertain | preserve the original operation; inspect an uncertain generation before creating another |
| 6 | replayed | the earlier result stands |
| 10 | owner signature needed | the owner opens the printed link |
| 11 | waiting for owner input | stop and report the prompt |
| 12 | pending | `allowit status ID --wait 60s` |
| 20 | denied by the policy | do not retry |

## Output

Text is the default. Add `--json` for JSON. Data goes to stdout, including an
owner signing link, which is the `signingUrl` field. Hints and request IDs go
to stderr.
