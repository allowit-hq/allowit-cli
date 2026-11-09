//! CLI presentation of the SDK's compiled rail inventory.
use crate::error::Result;
pub(crate) use allowit_sdk::routing::{Family, RailInfo, all, with_network};

pub(crate) fn info(network: &str) -> Result<&'static RailInfo> {
    // The SDK inventory lookup fails only when this build has no matching rail.
    allowit_sdk::routing::info(network).map_err(|mut error| {
        error.message.push_str("; run allowit rail ls");
        error
    })
}
pub(crate) fn normalize(network: &str) -> Result<&'static str> {
    info(network).map(|rail| rail.id)
}
pub(crate) fn family(network: &str) -> Result<Family> {
    info(network).map(|rail| rail.family)
}
