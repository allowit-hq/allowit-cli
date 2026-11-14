//! `allowit guide`: the agent skill, which is the guide for agents.
use crate::{ctx::Ctx, error::Result};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit guide
  allowit guide > ALLOWIT.md";

/// The agent skill that this repository ships. One text serves the skill
/// file and this command.
const SKILL: &str = include_str!("../../../skills/allowit/SKILL.md");

/// The skill without its front matter, with the example rail in place of
/// `<rail>`.
fn guide(rail: &str) -> String {
    let body = SKILL
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .map_or(SKILL, |(_, body)| body);
    body.trim_start().replace("<rail>", rail)
}

/// Print the agent guide: the commands in the order an agent uses them
#[derive(clap::Args, Debug)]
pub(crate) struct Args {}

pub(crate) fn run(_: Args, cx: &mut Ctx) -> Result<i32> {
    cx.out.raw = true;
    // The example rail of help: the selected rail, or the first registered one.
    let rail = super::examples::render("__ALLOWIT_EXAMPLE_RAIL__", &cx.global);
    let rail = if rail.is_empty() {
        "RAIL".to_owned()
    } else {
        rail
    };
    cx.out.stdout.push_str(&guide(&rail));
    Ok(0)
}

#[cfg(test)]
mod tests {
    /// Every token of the skill is paid on every agent run.
    #[test]
    fn the_skill_is_short_and_covers_the_agent_journey() {
        let guide = super::guide("example:testnet");
        assert!(!guide.starts_with("---"), "front matter is not guide text");
        assert!(guide.starts_with("# Pay within an Allowit policy"));
        assert!(guide.split_whitespace().count() <= 420, "the skill grew");
        for part in [
            "install.sh",
            "allowit POLICY_ID help",
            "allowit rail use example:testnet",
            "allowit keys generate agent",
            "allowit keys fund agent",
            "allowit rail trust ORIGIN KEY",
            "allowit wallet import --policy POLICY_ID",
            "allowit exec --op transfer",
            "allowit status ID --wait 60s",
            "allowit history",
            "- 20: the policy denied the request",
            "pay from your own account",
            "keys export",
        ] {
            assert!(guide.contains(part), "{part}");
        }
        // The skill file carries the front matter that a skill loader reads.
        assert!(super::SKILL.starts_with("---\nname: allowit\ndescription: "));
    }
}
