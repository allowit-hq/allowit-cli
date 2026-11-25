//! What a policy wallet holds of its asset now, from the selected rail's `balance` function: one
//! read-only observation of the network, the same for every rail that registers it. The read
//! needs no key, no server and no credit, and it moves nothing. `wallet show` prints it beside
//! the limits, so an agent that keeps an amount in the wallet reads it without the owner.
use crate::{
    ctx::Ctx,
    error::{Error, Result},
    routing::RailInfo,
};
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

/// The longest decimal scale that an amount of base units can have.
const MAX_DECIMALS: u8 = 38;

/// The JSON field and the text of an observation: what the wallet can send, in display units
/// with the wallet's asset symbol, its base units, the whole holding when the rail reports more
/// (`total`), and when and where it was read. The rail names the scale: an observation without
/// a balance or a scale in base units is no balance.
pub(crate) fn shown(seen: &Value, symbol: &str) -> Result<(Value, String)> {
    let units = |field: &str| -> Option<u128> { seen[field].as_str()?.parse().ok() };
    let unread =
        |what: &str| Error::config(format!("the rail's balance observation has no {what}"));
    let balance = units("balance").ok_or_else(|| unread("balance in base units"))?;
    let decimals = seen["decimals"]
        .as_u64()
        .and_then(|d| u8::try_from(d).ok())
        .filter(|d| *d <= MAX_DECIMALS)
        .ok_or_else(|| unread("decimals"))?;
    let amount = units_decimal(balance, decimals);
    let mut field = json!({
        "amount": amount,
        "asset": symbol,
        "units": balance.to_string(),
        "observedAt": seen["observedAt"],
        "source": seen["source"],
    });
    let mut text = format!("{amount} {symbol}");
    if let Some(total) = units("total").filter(|total| *total > balance) {
        let held = units_decimal(total, decimals);
        field["total"] = held.as_str().into();
        text = format!("{text} can be sent, of {held} {symbol} held");
    }
    Ok((field, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_balance_reads_in_display_units_of_the_wallet_asset() {
        let seen = json!({"asset": "CASSET", "assetCode": "COIN", "holder": "CWALLET",
            "balance": "5000000000", "total": "5000000000", "decimals": 7,
            "observedAt": 1_795_003_303u64, "source": "CASSET balance at ledger 100"});
        let (field, text) = shown(&seen, "COIN").unwrap();
        assert_eq!(text, "500 COIN");
        assert_eq!(
            field,
            json!({"amount": "500", "asset": "COIN", "units": "5000000000",
                "observedAt": 1_795_003_303u64, "source": "CASSET balance at ledger 100"})
        );
        let (_, text) = shown(&json!({"balance": "1", "decimals": 6}), "COIN").unwrap();
        assert_eq!(text, "0.000001 COIN");
        // An account that holds more than it can send.
        let (field, text) = shown(
            &json!({"balance": "2000000000", "total": "2030000000", "decimals": 7}),
            "COIN",
        )
        .unwrap();
        assert_eq!(text, "200 COIN can be sent, of 203 COIN held");
        assert_eq!(field["total"], "203");
    }

    #[test]
    fn an_observation_without_its_balance_or_scale_is_no_balance() {
        for seen in [
            json!({"decimals": 7}),
            json!({"balance": "12a", "decimals": 7}),
            json!({"balance": 12, "decimals": 7}),
            json!({"balance": "12"}),
            json!({"balance": "12", "decimals": "7"}),
            json!({"balance": "12", "decimals": 39}),
        ] {
            assert!(shown(&seen, "COIN").is_err(), "{seen}");
        }
    }
}
