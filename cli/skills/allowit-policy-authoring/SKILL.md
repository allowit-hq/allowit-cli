---
name: allowit-policy-authoring
description: Author, structurally check and compile portable Allowit Rust policies, inspect public wallet bindings and preserve exact operation identity during execution and recovery. For paying within a policy, use the allowit skill.
---

# Portable Allowit policies

Use this repository's `allowit` CLI and portable policy template.
Preserve the complete owner instructions and exact authored source across revisions.

A policy is one self-contained synchronous Rust file:

```rust
use allowit::prelude::*;

pub fn execute(ctx: &AllowitContext) -> allowit::Result {
    allowit::operations(ctx, &ctx.params[allowit::OPERATIONS])?;
    allowit::daily_spending_limit(ctx, &ctx.params[allowit::DAILY_SPENDING_LIMIT])?;
    allowit::payment_limit(ctx, &ctx.params[allowit::PAYMENT_LIMIT])?;
    Ok(())
}
```

Follow the [authoring order](../../../docs/policy-reference.md#authoring-order): declare allowed operations first, then preserve the owner's requested rule order.
Use canonical `allowit` rules for standard limits. Read owner-approved configuration through `ctx.params` and the documented keys.
Amounts use exact native base units. Missing or invalid required params deny execution.
The host and rail supply authenticated request facts, accounting and enrichment.
Caller-provided claims cannot replace trusted state, native authority or verified settlement.

Run `allowit policy check POLICY.rs` to inspect the exact source digest, leading canonical rules and state-write shape.
This command validates source structure with the profile of the wallet kind that holds the policy. The server's gate checks every policy with the server profile first, so a policy that this profile refuses, such as one over 16000 bytes, is refused for every kind. The server enforces the whole policy of a keypair wallet (`--kind native`) and keeps its params: the check uses the server profile, which admits the server functions and the text of an owner param read with `as_str()`. A Simple or Standard wallet holds its source on chain: the check uses the portable profile, also for a policy that calls a server function, which the server evaluates before it co-signs; such a policy states its text in the source. A Custom wallet (`--kind custom`) takes one source with the server profile: the server evaluates all of it, and the wallet contract compiles its on-chain part, the steps that the chain can check by itself. The result states that part in `onchain`: the checks that stay, the lines that left and each check that only the server enforces. A warning names each rule that runs only on the server because it is in a branch that depends on a server function; a rule that applies to every request is called before the server functions and outside such a branch, with the values that it reads, and a rule that applies only in that branch stays there and runs on the server. The result also names the figures that the wallet keeps for the kept statements and, in `onchain.state`, each state key of the policy with its keeper. `--onchain` prints the on-chain part itself. Without `--kind`, a policy that writes state is checked as a Custom wallet's policy, and any other policy that only the server profile admits as a keypair wallet's policy. A policy that writes state is refused for every kind but Custom, also when the check resolves the kind without `--kind`. For a Custom wallet the rail's own check also runs, as its build would: for example, its contract stores at most eight state keys. The result names the profile and the server functions that the policy calls, and the placement: `server` for a keypair wallet; for a Custom wallet `contract` when the contract runs the whole source and `contract and server` when a part leaves; none for a Simple or Standard wallet, whose installation binds the rules that its contract runs. Add `--compile` to also type-check through the pinned Core helper and portable Std library.
A policy for a Custom wallet may keep its own state with `allowit::get_state(ctx, "key")` and `allowit::set_state(ctx, "key", value)`. Each key has one keeper. The wallet contract (`chain`) keeps a key when a statement that uses it stays in the on-chain part and no statement that writes it leaves. The server (`server`) keeps every other key; then every statement that uses the key runs only on the server. No policy reads or writes the stored day `day`; read a daily figure such as `day_spent` instead. Store as little as possible, in this order: no state; a figure that a canonical rule keeps (`call_limit`, `lifetime_spending_limit`, `daily_spending_limit`, `daily_volume_limit`) or a value that the request gives; then one small number or bool under one short key. A key is a string literal of 1 to 32 ASCII letters, digits or `_`; a key that starts with `day_` lives one daily window. Write a value that does not depend on a server function before the first server function, so that the wallet contract keeps it. A statement that only the server runs is evaluated once, when the server prepares the operation, and the ledger applies the operation later. So such a statement never depends on a value that the ledger judges differently at the settlement, and `policy check` refuses a source that breaks this: (1) the time of the request, `ctx.request.now`, or the value or the outcome of `allowit::expires_at`; (2) a daily value that the wallet contract keeps, which is a `day_` key whose keeper is `chain`, or a daily figure that the on-chain part reads or checks (`onchain.figures`), read with `allowit::get_state` or through any call of its daily rule. A value reaches a statement by a read, by a name bound from it, by a branch or a loop on it, or by an approval before the statement. The refusal names the value and the line. Move the check before the first server function, so that the contract runs it too; or keep a `day_` key on the server by a write that depends on the server function; or keep a daily figure on the server by leaving no rule and no read of it in the on-chain part. No policy writes a standard figure (`spent`, `unpriced`, `calls`, `day`, `day_spent`, `day_total`, `day_unpriced`).
A decision question never judges an amount, an address, a price or a date: a decision model passes such mismatches, and a rule checks them exactly (`payment_limit`, `destinations`, `expires_at` or custom code with `ctx.request.amount` or `ctx.request.now`). The check warns on stderr when a question that the source states as a literal asks one; a question read from a param is not checked. The warning leaves the result unchanged.
`--compile` requires rustc. Missing tools, compilation failures and runtime limits stop the check.
`cargo test --locked -p allowit-policy-template` also type-checks the template as ordinary Rust.
A local build does not authorize publication, signing or spending.

The shared source gate admits only the portable vocabulary. Keep file access, environment access, unsafe code, foreign dependencies, floating-point values and pointer-width behavior out of authored source.
Custom state uses the admitted `get_state` and `set_state` interfaces. Simple and Standard wallets reject bespoke state writes.
Classic custody runs canonical-only source in full. It takes any other source, such as one that calls a server function, only with the server's evaluation of each operation, and it rejects bespoke state writes. The native installation binds the complete source, typed params and selected policy tier.

## Requests through the selected adapter

Run `allowit guide` for the agent journey; the [CLI reference](../../README.md) holds the owner and build commands.
Inspect `allowit rail ls --json` and `allowit rail show RAIL --json`; use registered identifiers and advertised capabilities.
Generate or import the local key, then select it with `allowit keys use NAME`.
Follow the selected rail's vendor plugin documentation for public wallet profiles and native input formats.
Read [known issues](../../../known_issues.md) before using a profile workaround.
`ALLOWIT_URL` selects the server origin. Public profiles contain no signing material.
The selected key must match the profile before any request or signature.

Use `wallet show` and `policy show --source` to inspect the binding. `walletKind` names the bound wallet kind. Params print in human units; `paramsDisplay` in JSON holds the same values.
Run `allowit policy kinds` for the four wallet kinds of the rail: `native`, `simple`, `standard` and `custom`. It marks the recommended kind and states for each kind the sellers that it pays and its network fee.
Recommend `native`: the agent's own wallet pays every x402 seller, and the seller's facilitator pays the network fee. `simple`, `standard` and `custom` are contract wallets. They pay only sellers that accept a settled payment as proof, and each payment costs more in network fee. Choose `standard` or `custom` when the owner asks for rules that the chain enforces.
Pass the owner's choice to `policy generate --kind KIND`. Do not choose a kind for the owner.
For `native`, the owner first creates and funds the agent's wallet account with `wallet setup --amount AMOUNT`, then drafts with `policy generate --kind native --installation ID`.
Use `exec` for the exact operation and retain its `--request-id`.
After an uncertain result, read `status` or repeat the same intent with the same ID. Do not create replacement bytes or a new spending request to resolve uncertainty.
A failed enclosing transaction does not prove that its retained authorization cannot execute elsewhere.
Apply the selected rail's authorization and verified-receipt semantics before resubmission.
Read verified native receipts before treating a payment as settled.
A settled `exec` or `status` result shows the transaction, `height`, `fee` and `explorer` link. Report amounts with the unit that the result prints.
A limit for each payment has no counter: it prints `-` for spent and remaining, and JSON lists it under `paymentLimits`.
`wallet show` prints the limits of a wallet before its first payment.
For the `custom` kind, run `policy build POLICY_ID --owner-key NAME --origin ORIGIN --out wallet.wasm` before `policy deploy`, and pass that file as `--executable`. The build hands the code to the server, which uploads it, and prints its code hash. `policy deploy --amount AMOUNT` creates and funds the wallet with one owner signature; a wallet of another asset than XLM also takes `--fee-asset-amount`.
`policy deploy` and `policy fund` print the hash, ledger, status and explorer link of each owner transaction.
Record `allowit --version` with a run: it names the git revision that built the binary.
A typed server refusal prints `error: CODE: MESSAGE`. Report the code; do not retry a refusal with exit 4.

Owner signing remains with the owner. Use `wallet setup` and the `policy deploy`, `fund`, `activate` and `export` owner workflow; each step binds its exact public intent.
