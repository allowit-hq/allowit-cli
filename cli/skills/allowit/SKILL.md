---
name: allowit
description: Author, structurally check and compile portable Allowit Rust policies, inspect public wallet bindings and preserve exact operation identity during execution and recovery.
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
This command validates source structure. Run `allowit policy build POLICY.rs -o wallet.rs` to type-check through the pinned Core helper and portable Std library before emitting source.
Python and rustc are required. Missing tools, compilation failures and runtime limits stop the build.
`cargo test --locked -p allowit-policy-template` also type-checks the template as ordinary Rust.
A local build does not authorize publication, signing or spending.

The shared source gate admits only the portable vocabulary. Keep file access, environment access, unsafe code, foreign dependencies, floating-point values and pointer-width behavior out of authored source.
Custom state uses the admitted `get_state` and `set_state` interfaces. Simple and Common wallets reject bespoke state writes.
Classic custody requires canonical-only source. The native installation binds the complete source, typed params and selected policy tier.

## Requests through the selected adapter

Run `allowit guide` for the command sequence and the build/configuration references.
Inspect `allowit rail ls --json` and `allowit rail show RAIL --json`; use registered identifiers and advertised capabilities.
Generate or import the local key, then select it with `allowit keys use NAME`.
Follow the selected adapter's [configuration reference](../../src/vendors/) for public wallet profiles and native input formats.
Read [known issues](../../../known_issues.md) before using a profile workaround.
`ALLOWIT_URL` selects the server origin. Public profiles contain no signing material.
The selected key must match the profile before any request or signature.

Use `wallet show` and `policy show --source` to inspect the binding.
Use `exec` for the exact operation and retain its `--request-id`.
After an uncertain result, read `status` or repeat the same intent with the same ID. Do not create replacement bytes or a new spending request to resolve uncertainty.
A failed enclosing transaction does not prove that its retained authorization cannot execute elsewhere.
Apply the selected rail's authorization and verified-receipt semantics before resubmission.
Read verified native receipts before treating a payment as settled.

Owner signing remains with the owner. The hidden `wallet configure` and `wallet approve` continuations retain their exact public intent and print a browser signing link.
Review that bound intent in the browser. An expired configure continuation remains available for recovery and cannot become a fresh authorization under the same ID.
