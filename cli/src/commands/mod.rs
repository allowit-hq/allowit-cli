//! The command tree. Each verb owns its arguments and terminal interaction.
//! SDK engine supplies shared execution and recovery workflows.
mod call;
mod completion;
mod config;
mod examples;
pub(crate) mod exec;
mod guide;
mod history;
mod keys;
mod mcp;
mod policy;
pub(crate) mod policy_help;
mod rail;
mod skills;
mod status;
mod version;
mod wallet;

use crate::{VERSION_LINE, ctx::Ctx, error::Result};
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};
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
  allowit POLICY_ID help
  allowit rail use __ALLOWIT_EXAMPLE_RAIL__
  allowit keys generate agent
  allowit exec --op transfer --to DESTINATION --amount ASSET:5
  allowit status pay-0001 --wait 60s";

const EXIT_CODES: &str = "\
Exit codes:
  0 ok, 1 internal error, 2 usage error, 3 configuration or not supported on
  this rail, 4 rejected by the contract or refused by the server, 5 uncertain
  (retry with the same ID), 6 replayed, 7 network unavailable (retry with the
  same ID), 8 seller failed, 9 price above --max-price (nothing paid), 10 owner
  signature needed, 11 waiting for owner input, 12 pending, 20 denied by the
  policy, 130 interrupted";

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
    /// Print JSON instead of text
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
    /// True when `--config-dir` is on the command line, not from `ALLOWIT_CONFIG_DIR`.
    #[arg(skip)]
    pub config_dir_flag: bool,
}

#[derive(Parser, Debug)]
#[command(
    name = "allowit",
    about = "AllowIt: policy wallets for agents. Go on. On your terms.",
    version = VERSION_LINE,
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
    #[command(subcommand, before_help = skills::EXAMPLES)]
    Skills(skills::Command),
    #[command(before_help = call::CALL_EXAMPLES)]
    Call(call::CallArgs),
    #[command(visible_alias = "http", before_help = call::CURL_EXAMPLES)]
    Curl(call::CurlArgs),
    #[command(before_help = mcp::EXAMPLES)]
    Mcp(mcp::Args),
    #[command(before_help = completion::EXAMPLES)]
    Completion(completion::Args),
    #[command(before_help = guide::EXAMPLES)]
    Guide(guide::Args),
    #[command(before_help = version::EXAMPLES)]
    Version(version::Args),
    /// The command behind `allowit POLICY_ID help`
    #[command(name = "policy-help", hide = true)]
    PolicyHelp(policy_help::Args),
}

/// Rewrites `allowit [FLAGS] POLICY_ID help` to the hidden `policy-help`
/// command. The owner copies that line from the app, so the policy id is the
/// first word. A word that is a command name is never an id, and an id can
/// start with `-`, so the id travels as `--id=POLICY_ID`.
pub(crate) fn policy_help_argv(argv: &[String]) -> Option<Vec<String>> {
    const FLAGS: &[&str] = &[
        "--json",
        "-y",
        "--yes",
        "-q",
        "--quiet",
        "-v",
        "--verbose",
        "--no-color",
    ];
    let mut at = 1;
    while at < argv.len() {
        let token = argv[at].as_str();
        if VALUE_FLAGS.contains(&token) {
            at += 2;
        } else if FLAGS.contains(&token)
            || VALUE_FLAGS
                .iter()
                .any(|flag| token.starts_with(&format!("{flag}=")))
        {
            at += 1;
        } else {
            break;
        }
    }
    let id = argv.get(at)?;
    if argv.get(at + 1).map(String::as_str) != Some("help")
        || !policy_help::is_policy_id(id)
        || command()
            .get_subcommands()
            .any(|c| c.get_name() == id || c.get_all_aliases().any(|alias| alias == id))
    {
        return None;
    }
    let mut out = argv[..at].to_vec();
    out.push("policy-help".into());
    out.push(format!("--id={id}"));
    out.extend_from_slice(&argv[at + 2..]);
    Some(out)
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
        let examples = cmd.get_before_help().map(ToString::to_string);
        // `policy generate` explains every wallet kind with the SDK's text.
        // The section is literal template text, as the examples are: clap
        // never wraps it, so each explanation stays one line. This runs in
        // the tree walk, which keeps the order of the commands.
        let kinds = examples.as_deref() == Some(policy::GENERATE_EXAMPLES);
        let template = match (&examples, kinds) {
            (Some(examples), true) => format!(
                "{}\n\n{}\n",
                examples_template(examples),
                crate::kinds::help_section()
            ),
            (Some(examples), false) => examples_template(examples),
            (None, _) => TEMPLATE.to_owned(),
        };
        let cmd = if kinds {
            cmd.mut_arg("kind", |kind| kind.help(crate::kinds::flag_help()))
        } else {
            cmd
        };
        let cmd = cmd.help_template(template).mut_subcommands(layout);
        // A bare group prints its help and changes nothing.
        if groups && cmd.get_name() != "params" && cmd.get_name() != "allowit" {
            cmd.subcommand_required(true).arg_required_else_help(true)
        } else {
            cmd
        }
    }
    layout(Cli::command()).mut_subcommand("exec", |exec| {
        let ops = compiled_ops();
        if ops.is_empty() {
            return exec;
        }
        exec.mut_arg("op", |op| {
            op.help(format!(
                "{}; rail show lists the selected rail's operations",
                ops.join(", ")
            ))
        })
    })
}

/// The operations that the compiled rails admit, in build order. `exec`
/// help names these, so it never offers an operation that no rail serves.
fn compiled_ops() -> Vec<&'static str> {
    let mut ops = vec![];
    for rail in crate::routing::all() {
        for op in crate::rails::rail(rail).caps().ops {
            if !ops.contains(op) {
                ops.push(*op);
            }
        }
    }
    ops
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
            let config_dir_flag = from_flag("config_dir");
            match Cli::from_arg_matches_mut(&mut matches) {
                Ok(mut cli) => {
                    cli.global.rail_flag = rail_flag;
                    cli.global.wallet_flag = wallet_flag;
                    cli.global.config_dir_flag = config_dir_flag;
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
        Command::Skills(c) => skills::run(c, cx),
        Command::Call(a) => call::run_call(a, cx),
        Command::Curl(a) => call::run_curl(a, cx),
        Command::Mcp(a) => mcp::run(a, cx),
        Command::Completion(a) => completion::run(a, cx),
        Command::Guide(a) => guide::run(a, cx),
        Command::Version(a) => version::run(a, cx),
        Command::PolicyHelp(a) => policy_help::run(a, cx),
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
            "--json policy check policy.rs --compile",
            "rail show example:mainnet --rail example:testnet",
            "rail use example:testnet --json",
            "exec --op transfer --to G --amount 1 --wallet C --rail example:testnet",
            "--json status pay-0001 --wait 5s",
        ] {
            assert!(
                matches!(parse(&argv(line), false), Parsed::Run(_)),
                "{line}"
            );
        }
        let Parsed::Run(cli) = parse(
            &argv("rail show example:mainnet --rail example:testnet"),
            false,
        ) else {
            unreachable!()
        };
        assert_eq!(cli.global.rail.as_deref(), Some("example:testnet"));
        assert!(cli.global.rail_flag);
        assert!(!cli.global.wallet_flag);
    }
    #[test]
    fn the_readme_lists_every_visible_command() {
        let readme = include_str!("../../README.md");
        let block = readme
            .split("## Commands\n\n```\n")
            .nth(1)
            .and_then(|rest| rest.split("\n```").next())
            .expect("cli/README.md has a Commands block");
        fn walk(cmd: &clap::Command, prefix: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
            let visible: Vec<_> = cmd
                .get_subcommands()
                .filter(|c| !c.is_hide_set() && c.get_name() != "help")
                .collect();
            if visible.is_empty() && !prefix.is_empty() {
                out.push(prefix.clone());
            }
            for sub in visible {
                prefix.push(sub.get_name().to_owned());
                walk(sub, prefix, out);
                prefix.pop();
            }
        }
        let mut leaves = vec![];
        let mut root = command();
        root.build();
        walk(&root, &mut vec![], &mut leaves);
        let missing: Vec<String> = leaves
            .iter()
            .map(|path| path.join(" "))
            .filter(|path| {
                let (group, name) = path.rsplit_once(' ').unwrap_or(("", path));
                let lead = format!("allowit {group}").trim_end().to_owned();
                // One line may list several commands: `allowit keys ls | show NAME`.
                !block.lines().any(|line| {
                    line.strip_prefix(&lead).is_some_and(|rest| {
                        let mut words = rest.split(|c: char| c.is_whitespace() || c == '|');
                        match (words.next(), group.is_empty()) {
                            (Some(""), true) => words.next() == Some(name),
                            (Some(""), false) => words.any(|word| word == name),
                            _ => false,
                        }
                    })
                })
            })
            .collect();
        assert!(missing.is_empty(), "cli/README.md omits {missing:?}");
    }
    #[test]
    fn a_policy_id_before_help_selects_the_policy_help_command() {
        let argv = |s: &str| {
            std::iter::once("allowit")
                .chain(s.split(' '))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let id = "Xg7IxRXCr2MhVdRrTFoYKeAw2iV4u463";
        for (line, expected) in [
            (format!("{id} help"), format!("policy-help --id={id}")),
            (
                format!("{id} help --json"),
                format!("policy-help --id={id} --json"),
            ),
            (
                format!("--json -q {id} help"),
                format!("--json -q policy-help --id={id}"),
            ),
            (
                format!("--rail example:testnet --config-dir=/tmp/x {id} help"),
                format!("--rail example:testnet --config-dir=/tmp/x policy-help --id={id}"),
            ),
            // An id can start with a hyphen.
            (
                "-1WagPbql_eK8CwzC4dxHJDMtC5wilVg help".to_owned(),
                "policy-help --id=-1WagPbql_eK8CwzC4dxHJDMtC5wilVg".to_owned(),
            ),
        ] {
            assert_eq!(
                policy_help_argv(&argv(&line)),
                Some(argv(&expected)),
                "{line}"
            );
            assert!(
                matches!(parse(&argv(&expected), false), Parsed::Run(_)),
                "{expected}"
            );
        }
        // Help of a command, plain help and other words stay as they are.
        for line in [
            "help",
            "keys help",
            "completion help",
            "wallet help",
            "help keys",
            "short help",
            "--json help",
            id,
            &format!("{id} show"),
            &format!("keys {id} help"),
        ] {
            assert_eq!(policy_help_argv(&argv(line)), None, "{line}");
        }
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
