//! `allowit guide`: the step-by-step guide for agents.
use crate::{ctx::Ctx, error::Result};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit guide
  allowit guide > ALLOWIT.md";

const GUIDE: &str = include_str!("guide.md");

/// Print the agent guide: the commands in the order an agent uses them
#[derive(clap::Args, Debug)]
pub(crate) struct Args {}

pub(crate) fn run(_: Args, cx: &mut Ctx) -> Result<i32> {
    cx.out.raw = true;
    cx.out
        .stdout
        .push_str(&super::examples::render(GUIDE, &cx.global));
    Ok(0)
}
