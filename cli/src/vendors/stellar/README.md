# Stellar CLI adapter

This adapter binds the common CLI commands to the [Stellar rail library](https://github.com/allowit-hq/allowit-contracts-stellar/tree/main/stellar/core).
The [workspace manifest](../../../../Cargo.toml) selects its release and the [lockfile](../../../../Cargo.lock) fixes the exact source.
[SDK registration](https://github.com/allowit-hq/allowit-sdk/blob/a2e50c4722fd82a7803c0bb1c78de5c2858a664f/engine/src/vendors/stellar.rs) maps the vendor library’s network metadata and capabilities into the executable.
Run `allowit rail ls --json` and `allowit rail show RAIL --json` for the installed build's networks, aliases and capabilities.

[Known issues](../../../../known_issues.md) describes compatibility limits for older public exports.

## Public wallet profile

`policy export` verifies the completed activation and saves the public profile in the shared SDK cache.
For a public export from another device, run `wallet import FILE --rpc-url URL`.
Native imports require the existing administrator signatures over the exact source, parameters and wallet binding.
Configure the independently verified server public identity with the common `rail trust` command first.
The SDK checks those signatures against current account authority.
Policy-wallet imports check the installed code, source, parameters and roles through independent RPC.

The profile contains no signing material. Import and export do not change the selected agent key.
`wallet show`, `policy show`, `exec` and `status` use the same cache resolver.
`wallet use ADDRESS --profile VERSION` selects an exact cached version when several profiles match.
Selecting an address does not discover source or import a profile.
`ALLOWIT_URL` selects the server origin for export and import. The cached profile retains that origin for execution.

## Keys and asset amounts

The adapter uses Ed25519 account keys. Public account addresses use G strkeys. Policy contracts use C strkeys.
Private key import/export uses Stellar S seeds. The common CLI keeps them in its private key store.
`keys fund` uses Friendbot only on supported Testnet networks, as advertised by `rail show`.

Amounts use `CODE:DECIMAL`, `CODE:ISSUER:DECIMAL`, `C…:DECIMAL` (an asset contract) or integer base units.
`XLM` and `native` identify the same asset. Native asset amounts use seven decimal places.

## Agent requests

The profile and installed asset determine the typed native or SEP-41 transfer.
Policy wallets also accept SEP-56 vault deposits and redemptions with `--token-out` minimums.
Use `rail show` to inspect the selected network's operations and available commands.
`--build-only` prints the typed request and sends nothing.

Fresh signing revalidates the prepared operation through `--rpc-url URL` or the rail's `rpc_url` setting.
This RPC receives only public observations and unsigned simulation bytes.
Fresh Native signing also checks source consent, current native roles and the exact source account through this RPC.
Original signed recovery skips current source, role and cache admission. It still requires an original-packet observation from the configured settlement service.
Shared SDK engine retains the original operation, submits it and recovers verified receipts.
The adapter supplies native rules. It does not own a separate transport retry or delivery coordinator.

## Owner authoring and signing

Use the wallet profile advertised by the owner's server, for example `allowit-stellar-policy-wallet-v1`.
The [CLI owner-generation instructions](../../../README.md#generate-an-owner-policy) describe the common command.
Generation produces a draft. It does not deploy, fund or activate a wallet.

The hidden `wallet configure` and `wallet approve` commands produce browser signing links for Stellar Testnet.
`configure` binds signer-setting operations of a funded classic account.
`approve` binds policy source, revision, typed params, tier, rules, network and wallet subject.
Both retain their original intent in a working-directory journal and exit 10.
