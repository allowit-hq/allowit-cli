# Portable policies

A policy is one self-contained synchronous Rust file with this entrypoint:

```rust
use allowit::prelude::*;

pub fn execute(ctx: &AllowitContext) -> allowit::Result {
    allowit::operations(ctx, &ctx.params[allowit::OPERATIONS])?;
    allowit::daily_spending_limit(ctx, &ctx.params[allowit::DAILY_SPENDING_LIMIT])?;
    allowit::payment_limit(ctx, &ctx.params[allowit::PAYMENT_LIMIT])?;
    Ok(())
}
```

The `allowit` package lives in [Allowit core](https://github.com/allowit-hq/allowit-core/tree/v0.3.1/crates/allowit).
Its `std` feature enables host evaluation. Native adapters bind the portable source to contract backends.
The shared Core source gate admits only the portable vocabulary and one `execute` entrypoint.
Host compilation uses the same exact Core revision as the gate and portable library.

## Authoring order

Start the policy's rule sequence with `allowit::operations(ctx, &ctx.params[allowit::OPERATIONS])?`.
Its second argument supplies the owner's allowed operation set.
The owner chooses the operation set. The generator places this rule first, then follows the order of the owner's remaining requirements.
Server generation owns this order. Consumers display it unchanged and preserve explicit owner edits to parameter values.

## Source and state

Policy source can use the admitted `allowit::` and `core` paths, fixed-width integers, local functions and portable control flow.
The gate rejects file and environment access, unsafe code, foreign dependencies, pointer-width values, floating-point values and macros outside its allowlist.
A valid source shape still needs Rust type-checking and native compilation before installation.

`ctx.request` contains authenticated operation facts. `ctx.params` contains owner-approved configuration.
A source reads canonical params by their `allowit` keys. Missing or invalid required values deny execution.
Amounts use exact integer base units; the native asset binding supplies their meaning.

The canonical rules cover payment limits, lifetime and daily spending, daily volume, call count, destinations, tokens, expiry and operations.
The shared library checks state and performs accounting around policy evaluation.
Custom policies use the admitted `get_state` and `set_state` interfaces for bespoke state.
Trusted hosts supply enrichment and application accounting; request JSON cannot replace those bindings.

## Source commands

```sh
allowit policy check policy.rs
allowit policy build policy.rs -o wallet.rs
allowit policy show --source
```

`check` validates the source structure and prints its SHA-256 digest, leading canonical rule calls, whether the source is canonical-only and whether it writes state.
`build` type-checks the source through the embedded Core Python helper and exact portable Std library, then emits it. Python and rustc are required. Compilation has wall-time and output limits. Neither command authorizes a transaction or establishes settlement.
The [policy template](../cli/templates/policy/) type-checks authored source as an ordinary Rust crate.

## Native binding

The Stellar rail derives the exact native request, effect and fee bounds, authorizers, simulation and settlement rules.
Its Simple, Common and Custom wallet implementations bind source, code and owner configuration.
Common uses the owner-approved leading canonical rule calls. Custom compiles the complete portable source.
Simple requires full trusted host evaluation before the server authorizes execution.
Simple and Common reject bespoke state writes.

Classic custody requires a canonical-only policy because its current signing path has no compiled custom evaluator.
Historical compiler-language packets remain readable for verification and status recovery.
They cannot authorize new preparation, signatures or a replacement payment.

The server binds source and compiled code to the owner's approved record and supplies trusted context, its signing authority, reservations and durable accounting.
SDK engine preserves operation identity and exact signed bytes across execution and recovery.
Each caller retains its keys and storage authority.
