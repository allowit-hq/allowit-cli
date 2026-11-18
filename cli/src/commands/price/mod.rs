//! `allowit price`: what the selected rail's `price` evidence function observes now, the same
//! observation that a server policy reads with `allowit::price`. For a vault, the vault itself
//! names what one share pays, and the selected wallet's position comes with it. For two assets,
//! the rail's market names what selling an amount pays. The read is free: it needs no key, no
//! server and no credit, and it moves nothing.
use crate::{
    ctx::Ctx,
    error::{Error, Result},
    rails,
};
use allowit_sdk::input::asset::units_decimal;
use serde_json::{Value, json};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit price VAULT
  allowit price VAULT --json
  allowit price VAULT --amount 500000000
  allowit price native USDC --amount 50000000";

/// Read a price now: what one share of a vault pays, with the wallet's shares and their worth,
/// or what selling an amount of one asset for another pays
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// A vault address, or the asset to sell
    pub from: String,
    /// The asset to receive; for a vault it may be omitted and must be the vault's asset
    pub to: Option<String>,
    /// INTEGER base units of FROM: shares of the vault, or the amount to sell [default for a
    /// vault: one share]
    #[arg(long, value_name = "UNITS")]
    pub amount: Option<String>,
    /// The account whose vault shares to value [default: the selected wallet]
    #[arg(long, value_name = "ADDR")]
    pub holder: Option<String>,
}

/// The `price` evidence function of every rail.
const FUNCTION: &str = "price";

pub(crate) fn run(a: Args, cx: &mut Ctx) -> Result<i32> {
    if let Some(amount) = &a.amount
        && (amount.is_empty()
            || amount.starts_with('0')
            || !amount.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(Error::usage(
            "--amount takes INTEGER base units, such as 10000000",
        ));
    }
    let rail = cx.rail()?;
    let holder = match a.holder {
        Some(holder) => {
            rails::address::check_wallet(rail.family, &holder)?;
            Some(holder)
        }
        None => cx.wallet(rail)?,
    };
    let mut args = json!({"network": rail.id, "from": a.from});
    if let Some(to) = &a.to {
        args["to"] = to.as_str().into();
    }
    if let Some(amount) = &a.amount {
        args["amount"] = amount.as_str().into();
    }
    if let Some(holder) = &holder {
        args["holder"] = holder.as_str().into();
    }
    let mut facts = json!({"network": rail.id});
    if let Some(rpc) = cx.rpc_url(rail)? {
        facts["rpc"] = rpc.into();
    }
    let plugin = allowit_sdk::plugins::for_network(rail.id)?;
    let seen = allowit_sdk::plugins::invoke_policy_function(plugin.id(), FUNCTION, &args, &facts)?;
    if cx.out.json {
        cx.out.json_value(&seen);
    } else {
        cx.out.table(&rows(&a.from, a.to.as_deref(), &seen));
    }
    Ok(0)
}

/// Base units of a decimal text field, or zero.
fn units(value: &Value) -> u128 {
    value.as_str().and_then(|t| t.parse().ok()).unwrap_or(0)
}

/// When the observation was made, as UTC time: the close time of the ledger it read.
fn when(seen: &Value) -> String {
    let Some(secs) = seen["observedAt"].as_i64().filter(|s| *s >= 0) else {
        return "unknown".into();
    };
    // Days since 1970-01-01 to a civil date (H. Hinnant, `civil_from_days`).
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// The text rows of an observation. Prices are scaled by 10^7.
fn rows(from: &str, to: Option<&str>, seen: &Value) -> Vec<Vec<String>> {
    let row = |k: &str, v: String| vec![k.to_owned(), v];
    let price = units_decimal(units(&seen["price"]), 7);
    let source = seen["source"].as_str().unwrap_or_default();
    let Some(vault) = seen.get("vault").filter(|v| v.is_object()) else {
        let to = to.unwrap_or("?");
        let mut out = vec![
            row("price", format!("1 {from} = {price} {to}")),
            row(
                "depth",
                format!(
                    "{} base units of {from}",
                    seen["depth"].as_str().unwrap_or("0")
                ),
            ),
        ];
        if seen["trades"].as_u64().unwrap_or(0) > 0 {
            let average = units_decimal(units(&seen["tradeAverage"]), 7);
            out.push(row(
                "trades",
                format!(
                    "{average} {to} on average over {} recent trades",
                    seen["trades"]
                ),
            ));
        }
        out.push(row("seen", format!("{} ({source})", when(seen))));
        return out;
    };
    let symbol = vault["assetCode"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| vault["asset"].as_str().unwrap_or("?").to_owned());
    let decimals = u8::try_from(vault["shareDecimals"].as_u64().unwrap_or(7)).unwrap_or(7);
    let shares = |field: &str| units_decimal(units(&vault[field]), decimals);
    let asset = |field: &str| units_decimal(units(&vault[field]), 7);
    let mut out = vec![
        row("vault", from.to_owned()),
        row("price", format!("1 share = {price} {symbol}")),
    ];
    if vault["shares"].as_str() != Some(&10u128.pow(u32::from(decimals)).to_string()) {
        out.push(row(
            "redeem",
            format!("{} shares pay {} {symbol}", shares("shares"), asset("paid")),
        ));
    }
    if !vault["held"].is_null() {
        out.push(row(
            "position",
            format!(
                "{} shares, worth {} {symbol}",
                shares("held"),
                asset("heldValue")
            ),
        ));
    }
    out.push(row("seen", format!("{} ({source})", when(seen))));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vault_observation_reads_as_its_share_price_and_the_position() {
        let seen = json!({
            "price": "9400000", "depth": "10000000", "tradeAverage": "0", "trades": 0,
            "observedAt": 1_795_003_303u64, "source": "CVAULT preview_redeem at ledger 5795931",
            "vault": {"asset": "CASSET", "assetCode": "COIN", "shareDecimals": 7,
                "shares": "10000000", "paid": "9400000", "held": "1000000000", "heldValue": "940000000"}
        });
        let text: Vec<String> = rows("CVAULT", None, &seen)
            .into_iter()
            .map(|r| r.join(" "))
            .collect();
        assert_eq!(
            text,
            [
                "vault CVAULT",
                "price 1 share = 0.94 COIN",
                "position 100 shares, worth 94 COIN",
                "seen 2026-11-18T12:01:43Z (CVAULT preview_redeem at ledger 5795931)",
            ]
        );
    }

    #[test]
    fn an_amount_of_shares_and_a_market_pair_read_as_what_they_pay() {
        let vault = json!({
            "price": "9600000", "observedAt": 0, "source": "s",
            "vault": {"asset": "CASSET", "assetCode": null, "shareDecimals": 7,
                "shares": "50000000", "paid": "48000000", "held": null}
        });
        let text: Vec<String> = rows("CVAULT", None, &vault)
            .into_iter()
            .map(|r| r.join(" "))
            .collect();
        assert_eq!(text[1], "price 1 share = 0.96 CASSET");
        assert_eq!(text[2], "redeem 5 shares pay 4.8 CASSET");
        let market = json!({"price": "1234567", "depth": "50000000", "tradeAverage": "1200000",
            "trades": 3, "observedAt": 0, "source": "market.example ledger 1"});
        let text: Vec<String> = rows("native", Some("USDC"), &market)
            .into_iter()
            .map(|r| r.join(" "))
            .collect();
        assert_eq!(text[0], "price 1 native = 0.1234567 USDC");
        assert_eq!(text[2], "trades 0.12 USDC on average over 3 recent trades");
        assert_eq!(
            text[3],
            "seen 1970-01-01T00:00:00Z (market.example ledger 1)"
        );
        assert_eq!(
            when(&json!({"observedAt": 951_782_400})),
            "2000-02-29T00:00:00Z"
        );
        assert_eq!(when(&json!({})), "unknown");
    }
}
