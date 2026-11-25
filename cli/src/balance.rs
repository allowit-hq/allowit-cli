//! What a policy wallet holds of its asset now, from the selected rail's `balance` function: one
//! read-only observation of the network, the same for every rail that registers it. The read
//! needs no key, no server and no credit, and it moves nothing. `wallet show` prints it beside
//! the limits, so an agent that keeps an amount in the wallet reads it without the owner.
use crate::{ctx::Ctx, error::Result, routing::RailInfo};
use allowit_sdk::input::asset::units_decimal;
use serde_json::{Value, json};

/// The rail function that answers a balance.
pub(crate) const FUNCTION: &str = "balance";

/// What `holder` holds of `asset` on the rail now. `None` when the rail registers no balance
/// read, so the caller shows no balance and no warning.
pub(crate) fn read(
    cx: &mut Ctx,
    rail: &RailInfo,
    holder: &str,
    asset: &str,
) -> Option<Result<Value>> {
    let plugin = allowit_sdk::plugins::for_network(rail.id).ok()?;
    allowit_sdk::plugins::policy_function(plugin.id(), FUNCTION)?;
    let mut read = || {
        let mut facts = json!({"network": rail.id});
        if let Some(rpc) = cx.rpc_url(rail)? {
            facts["rpc"] = rpc.into();
        }
        let args = json!({"network": rail.id, "holder": holder, "asset": asset});
        allowit_sdk::plugins::invoke_policy_function(plugin.id(), FUNCTION, &args, &facts)
    };
    Some(read())
}

/// The JSON field and the text of an observation: the amount in display units with the
/// wallet's asset symbol, the base units, and when and where it was read.
pub(crate) fn shown(seen: &Value, symbol: &str) -> (Value, String) {
    let units: u128 = seen["balance"]
        .as_str()
        .and_then(|t| t.parse().ok())
        .unwrap_or(0);
    let decimals = seen["decimals"]
        .as_u64()
        .and_then(|d| u8::try_from(d).ok())
        .unwrap_or(7);
    let amount = units_decimal(units, decimals);
    let field = json!({
        "amount": amount,
        "asset": symbol,
        "units": units.to_string(),
        "observedAt": seen["observedAt"],
        "source": seen["source"],
    });
    (field, format!("{amount} {symbol}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_balance_reads_in_display_units_of_the_wallet_asset() {
        let seen = json!({"asset": "CASSET", "assetCode": "COIN", "holder": "CWALLET",
            "balance": "5000000000", "decimals": 7, "observedAt": 1_795_003_303u64,
            "source": "CASSET balance at ledger 100"});
        let (field, text) = shown(&seen, "COIN");
        assert_eq!(text, "500 COIN");
        assert_eq!(
            field,
            json!({"amount": "500", "asset": "COIN", "units": "5000000000",
                "observedAt": 1_795_003_303u64, "source": "CASSET balance at ledger 100"})
        );
        let (_, text) = shown(&json!({"balance": "1", "decimals": 6}), "USDC");
        assert_eq!(text, "0.000001 USDC");
    }
}
