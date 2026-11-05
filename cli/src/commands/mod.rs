//! The command tree. Each verb owns its arguments and terminal interaction.
//! SDK engine supplies shared execution and recovery workflows.
mod completion;
mod config;
mod examples;
pub(crate) mod exec;
mod guide;
mod history;
mod keys;
mod policy;
mod rail;
mod status;
mod version;
mod wallet;

use crate::{VERSION, ctx::Ctx, error::Result};
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use std::{path::PathBuf, time::Duration};

/// Help layout: about, usage, examples, then commands and options.
const TEMPLATE: &str =
    "{about-with-newline}\n{usage-heading} {usage}\n\n{before-help}{all-args}{after-help}";

/// The help template of a node with examples. The examples are literal
/// template text, which clap never wraps, so a long example stays one line
/// that copies whole.
fn examples_template(examples: &str) -> String {
    format!(
        "{{about-with-newline}}\n{{usage-heading}} {{usage}}\n\n{examples}\n\n{{all-args}}{{after-help}}"
    )
}

const ROOT_EXAMPLES: &str = "\
Examples:
  allowit rail use __ALLOWIT_EXAMPLE_RAIL__
  allowit keys generate agent
  allowit exec --op transfer --to DESTINATION --amount ASSET:5
  allowit status pay-0001 --wait 60s";

const EXIT_CODES: &str = "\
Exit codes:
  0 ok, 1 internal error, 2 usage error, 3 configuration or not supported on
  this rail, 4 rejected by the contract, 5 uncertain (retry with the same ID),
  6 replayed, 10 owner signature needed, 11 waiting for owner input,
  12 pending, 20 denied by the policy, 130 interrupted";

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Format {
    Text,
    Json,
}

/// Flags that every command accepts.
#[derive(Args, Clone, Debug, Default)]
#[command(next_help_heading = "Global options")]
pub(crate) struct Global {
    /// Rail identifier or alias from rail ls
    #[arg(
        long,
        short = 'n',
        visible_alias = "network",
        env = "ALLOWIT_RAIL",
        hide_env_values = true,
        global = true,
        value_name = "RAIL"
    )]
    pub rail: Option<String>,
    /// Wallet address accepted by the selected rail
    #[arg(
        long,
        env = "ALLOWIT_WALLET",
        hide_env_values = true,
        global = true,
        value_name = "ADDR"
    )]
    pub wallet: Option<String>,
    /// Output format
    #[arg(long, value_enum, global = true, value_name = "FORMAT")]
    pub output: Option<Format>,
    /// Same as --output json
    #[arg(long, global = true)]
    pub json: bool,
    /// Confirm destructive actions without a prompt
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,
    /// Print no progress on stderr
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,
    /// Print the resolved rail, wallet and config directory on stderr
    #[arg(short, long, global = true)]
    pub verbose: bool,
    /// Disable color (NO_COLOR also disables it)
    #[arg(long, global = true)]
    pub no_color: bool,
    /// Config directory [default: $XDG_CONFIG_HOME/allowit or ~/.config/allowit]
    #[arg(
        long,
        env = "ALLOWIT_CONFIG_DIR",
        hide_env_values = true,
        global = true,
        value_name = "DIR"
    )]
    pub config_dir: Option<PathBuf>,
    /// RPC URL override for the selected rail
    #[arg(long, global = true, value_name = "URL")]
    pub rpc_url: Option<String>,
    /// True when `--rail` is on the command line, not from `ALLOWIT_RAIL`.
    #[arg(skip)]
    pub rail_flag: bool,
    /// True when `--wallet` is on the command line, not from `ALLOWIT_WALLET`.
    #[arg(skip)]
    pub wallet_flag: bool,
}

impl Global {
    /// JSON with `--json` or `--output json`; also with `NO_DNA` unless
    /// `--output text` asks for text.
    pub(crate) fn json_output(&self) -> bool {
        self.json
            || match self.output {
                Some(format) => format == Format::Json,
                None => crate::term::no_dna(),
            }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "allowit",
    about = "AllowIt: policy wallets for agents. Go on. On your terms.",
    version = VERSION,
    before_help = ROOT_EXAMPLES,
    after_help = EXIT_CODES,
    disable_version_flag = false
)]
pub(crate) struct Cli {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    #[command(subcommand, before_help = keys::EXAMPLES)]
    Keys(keys::Command),
    #[command(subcommand, before_help = rail::EXAMPLES)]
    Rail(rail::Command),
    #[command(subcommand, before_help = config::EXAMPLES)]
    Config(config::Command),
    #[command(subcommand, before_help = wallet::EXAMPLES)]
    Wallet(wallet::Command),
    #[command(subcommand, before_help = policy::EXAMPLES)]
    Policy(policy::Command),
    #[command(before_help = exec::EXAMPLES)]
    Exec(exec::Args),
    #[command(before_help = status::EXAMPLES)]
    Status(status::Args),
    #[command(before_help = history::EXAMPLES)]
    History(history::Args),
    #[command(before_help = completion::EXAMPLES)]
    Completion(completion::Args),
    #[command(before_help = guide::EXAMPLES)]
    Guide(guide::Args),
    #[command(before_help = version::EXAMPLES)]
    Version(version::Args),
}

/// Parses `60s`, `5m`, `1h30m`, `500ms` or `0`, at most 10 minutes.
pub(crate) fn parse_wait(s: &str) -> std::result::Result<Duration, String> {
    let invalid = || format!("{s:?} is not a duration such as 30s, 5m or 0");
    if s == "0" {
        return Ok(Duration::ZERO);
    }
    let re = regex::Regex::new(r"([0-9]+(?:\.[0-9]*)?|\.[0-9]+)(ms|s|m|h)").unwrap();
    let mut consumed = 0;
    let mut nanos = 0f64;
    for c in re.captures_iter(s) {
        let whole = c.get(0).unwrap();
        if whole.start() != consumed {
            return Err(invalid());
        }
        consumed = whole.end();
        let n = c[1].parse::<f64>().map_err(|_| invalid())?;
        nanos += n * match &c[2] {
            "ms" => 1e6,
            "s" => 1e9,
            "m" => 60e9,
            _ => 3600e9,
        };
    }
    if consumed != s.len() || consumed == 0 {
        return Err(invalid());
    }
    if nanos > 600e9 {
        return Err("the wait must be at most 10m".into());
    }
    Ok(Duration::from_nanos(nanos as u64))
}

/// The full command with the shared help layout on every node.
pub(crate) fn command() -> clap::Command {
    fn layout(cmd: clap::Command) -> clap::Command {
        let groups = cmd.get_subcommands().count() > 0;
        let template = match cmd.get_before_help() {
            Some(examples) => examples_template(&examples.to_string()),
            None => TEMPLATE.to_owned(),
        };
        let cmd = cmd.help_template(template).mut_subcommands(layout);
        // A bare group prints its help and changes nothing.
        if groups && cmd.get_name() != "params" && cmd.get_name() != "allowit" {
            cmd.subcommand_required(true).arg_required_else_help(true)
        } else {
            cmd
        }
    }
    layout(Cli::command())
}

pub(crate) enum Parsed {
    Run(Box<Cli>),
    /// Help or version: print to stdout and exit 0.
    Print(String),
    Error(clap::Error),
}

pub(crate) fn parse(argv: &[String], color: bool) -> Parsed {
    let mut cmd = command().color(if color {
        clap::ColorChoice::Auto
    } else {
        clap::ColorChoice::Never
    });
    match cmd.try_get_matches_from_mut(argv) {
        Ok(mut matches) => {
            let from_flag =
                |id: &str| matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine);
            let (rail_flag, wallet_flag) = (from_flag("rail"), from_flag("wallet"));
            match Cli::from_arg_matches_mut(&mut matches) {
                Ok(mut cli) => {
                    cli.global.rail_flag = rail_flag;
                    cli.global.wallet_flag = wallet_flag;
                    match cli.command {
                        None => Parsed::Print(examples::help(cmd.render_help().to_string(), argv)),
                        Some(_) => Parsed::Run(Box::new(cli)),
                    }
                }
                Err(e) => Parsed::Error(e),
            }
        }
        Err(e) => match e.kind() {
            clap::error::ErrorKind::DisplayHelp
            | clap::error::ErrorKind::DisplayVersion
            | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                Parsed::Print(examples::help(e.render().to_string(), argv))
            }
            // A bare group prints its help and changes nothing.
            clap::error::ErrorKind::MissingSubcommand => {
                let mut root = command().color(if color {
                    clap::ColorChoice::Auto
                } else {
                    clap::ColorChoice::Never
                });
                root.build();
                Parsed::Print(examples::help(
                    group(&mut root, argv).render_help().to_string(),
                    argv,
                ))
            }
            _ => Parsed::Error(e),
        },
    }
}

/// Value flags of the root command, whose values are not command names.
const VALUE_FLAGS: &[&str] = &[
    "--rail",
    "-n",
    "--network",
    "--wallet",
    "--output",
    "--config-dir",
    "--rpc-url",
];

/// The deepest command that the words of `argv` name.
fn group<'a>(root: &'a mut clap::Command, argv: &[String]) -> &'a mut clap::Command {
    let mut current = root;
    let mut skip = false;
    for token in argv.iter().skip(1) {
        if skip {
            skip = false;
            continue;
        }
        if token.starts_with('-') {
            skip = VALUE_FLAGS.contains(&token.as_str());
            continue;
        }
        let name = current
            .get_subcommands()
            .find(|c| c.get_name() == token || c.get_all_aliases().any(|a| a == token))
            .map(|c| c.get_name().to_owned());
        match name {
            Some(name) => current = current.find_subcommand_mut(name).expect("subcommand"),
            None => break,
        }
    }
    current
}

/// The visible subcommands at the point where an unknown command appeared.
pub(crate) fn available_commands(argv: &[String]) -> Vec<String> {
    let mut root = command();
    let current = group(&mut root, argv);
    current
        .get_subcommands()
        .filter(|c| !c.is_hide_set())
        .map(|c| c.get_name().to_owned())
        .collect()
}

/// Every command path, hidden aliases included, in tree order. The help
/// snapshot tests run `--help` on each one.
pub(crate) fn paths() -> Vec<Vec<String>> {
    fn walk(cmd: &clap::Command, prefix: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
        out.push(prefix.clone());
        for sub in cmd.get_subcommands() {
            prefix.push(sub.get_name().to_owned());
            walk(sub, prefix, out);
            prefix.pop();
        }
    }
    let mut out = vec![];
    walk(&command(), &mut vec![], &mut out);
    out
}

pub(crate) fn dispatch(command: Command, cx: &mut Ctx) -> Result<i32> {
    match command {
        Command::Keys(c) => keys::run(c, cx),
        Command::Rail(c) => rail::run(c, cx),
        Command::Config(c) => config::run(c, cx),
        Command::Wallet(c) => wallet::run(c, cx),
        Command::Policy(c) => policy::run(c, cx),
        Command::Exec(a) => exec::run(a, cx),
        Command::Status(a) => status::run(a, cx),
        Command::History(a) => history::run(a, cx),
        Command::Completion(a) => completion::run(a, cx),
        Command::Guide(a) => guide::run(a, cx),
        Command::Version(a) => version::run(a, cx),
    }
}

/// True for commands that need no selected rail, wallet or signing credential.
pub(crate) fn is_local(command: &Command) -> bool {
    matches!(
        command,
        Command::Completion(_) | Command::Guide(_) | Command::Version(_)
    ) || matches!(command, Command::Policy(p) if policy::is_local(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_command_tree_is_consistent() {
        command().debug_assert();
    }
    #[test]
    fn examples_never_wrap_at_any_width() {
        fn walk(cmd: &mut clap::Command, path: &str) {
            if let Some(examples) = cmd.get_before_help().map(|e| e.to_string()) {
                let help = cmd.clone().term_width(40).render_help().to_string();
                assert!(
                    help.contains(&format!("\n{examples}\n")),
                    "{path}: the examples changed in\n{help}"
                );
            }
            for sub in cmd.get_subcommands_mut() {
                let name = format!("{path} {}", sub.get_name());
                walk(sub, &name);
            }
        }
        let mut root = command();
        root.build();
        walk(&mut root, "allowit");
    }
    #[test]
    fn no_local_argument_reuses_a_global_argument_id() {
        let root = command();
        let globals: Vec<String> = root
            .get_arguments()
            .filter(|a| a.is_global_set())
            .map(|a| a.get_id().to_string())
            .collect();
        fn walk(cmd: &clap::Command, globals: &[String], path: &str) {
            for arg in cmd.get_arguments().filter(|a| !a.is_global_set()) {
                let id = arg.get_id().to_string();
                assert!(
                    !globals.contains(&id),
                    "{path}: argument {id} shadows a global flag"
                );
            }
            for sub in cmd.get_subcommands() {
                walk(sub, globals, &format!("{path} {}", sub.get_name()));
            }
        }
        for sub in root.get_subcommands() {
            walk(sub, &globals, sub.get_name());
        }
    }
    #[test]
    fn every_command_parses_with_global_flags_in_any_position() {
        let argv = |s: &str| {
            std::iter::once("allowit")
                .chain(s.split(' '))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        for line in [
            "keys use agent --json",
            "--output json policy build policy.rs -o out.rs",
            "rail show stellar:pubnet --rail stellar:testnet",
            "rail use stellar:testnet --json",
            "exec --op transfer --to G --amount 1 --wallet C --rail stellar:testnet",
            "--json status pay-0001 --wait 5s",
        ] {
            assert!(
                matches!(parse(&argv(line), false), Parsed::Run(_)),
                "{line}"
            );
        }
        let Parsed::Run(cli) = parse(
            &argv("rail show stellar:pubnet --rail stellar:testnet"),
            false,
        ) else {
            unreachable!()
        };
        assert_eq!(cli.global.rail.as_deref(), Some("stellar:testnet"));
        assert!(cli.global.rail_flag);
        assert!(!cli.global.wallet_flag);
    }
    #[test]
    fn waits_parse_and_are_bounded() {
        assert_eq!(parse_wait("60s").unwrap(), Duration::from_secs(60));
        assert_eq!(parse_wait("1m30s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_wait("0").unwrap(), Duration::ZERO);
        assert_eq!(parse_wait("500ms").unwrap(), Duration::from_millis(500));
        for bad in ["", "10", "-1s", "11m", "1d", "5 m"] {
            assert!(parse_wait(bad).is_err(), "{bad}");
        }
    }
}
