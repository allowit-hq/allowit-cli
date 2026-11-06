//! `allowit completion --shell SHELL`.
use crate::{ctx::Ctx, error::Result};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit completion --shell zsh > ~/.zfunc/_allowit
  allowit completion --shell bash > /etc/bash_completion.d/allowit";

/// Print a shell completion script
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// The shell
    #[arg(long, value_enum)]
    pub shell: clap_complete::Shell,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let mut buffer = Vec::new();
    clap_complete::generate(
        args.shell,
        &mut visible(&super::command()),
        "allowit",
        &mut buffer,
    );
    cx.out.raw = true;
    cx.out.stdout.push_str(&String::from_utf8_lossy(&buffer));
    Ok(0)
}

/// The command tree without hidden commands and options, which completion
/// must not offer.
fn visible(cmd: &clap::Command) -> clap::Command {
    // The process prints one script and exits, so the names may live forever.
    fn name(s: &str) -> &'static str {
        Box::leak(s.to_owned().into_boxed_str())
    }
    let mut out = clap::Command::new(name(cmd.get_name()))
        .visible_aliases(cmd.get_visible_aliases().map(name).collect::<Vec<_>>());
    if let Some(about) = cmd.get_about() {
        out = out.about(about.clone());
    }
    if let Some(version) = cmd.get_version() {
        out = out.version(name(version));
    }
    for arg in cmd.get_arguments().filter(|a| !a.is_hide_set()) {
        out = out.arg(arg.clone());
    }
    for sub in cmd.get_subcommands().filter(|s| !s.is_hide_set()) {
        out = out.subcommand(visible(sub));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completions_offer_no_hidden_command() {
        fn walk(cmd: &clap::Command) {
            for sub in cmd.get_subcommands() {
                assert!(!sub.is_hide_set(), "{}", sub.get_name());
                walk(sub);
            }
        }
        let tree = visible(&super::super::command());
        walk(&tree);
        tree.clone().debug_assert();
        for shell in [clap_complete::Shell::Bash, clap_complete::Shell::Zsh] {
            let mut buffer = Vec::new();
            clap_complete::generate(shell, &mut tree.clone(), "allowit", &mut buffer);
            let script = String::from_utf8(buffer).unwrap();
            for hidden in ["configure", "approve"] {
                assert!(!script.contains(hidden), "{shell}: {hidden}");
            }
            assert!(script.contains("generate") && script.contains("status"));
        }
    }
}
