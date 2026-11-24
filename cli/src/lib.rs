//! The `allowit` command. Argument parsing, terminal output and rail adapters
//! live here; engine requests and service composition come from
//! `allowit-sdk-engine`, which has no terminal I/O.
pub use allowit_sdk::{error, wallet};
/// This CLI's own release version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The git revision that built this binary: the short commit hash, with
/// `-dirty` when the working tree differed from that commit, or `unknown`.
/// The build script records it.
pub const REVISION: &str = env!("ALLOWIT_BUILD_REVISION");
/// What `--version` prints after the name: the release and the revision.
pub(crate) const VERSION_LINE: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("ALLOWIT_BUILD_REVISION"),
    ")"
);
pub(crate) use allowit_sdk::{config, output, request};
mod agent_read;
mod commands;
mod credits;
mod ctx;
mod exit;
mod explain;
mod kinds;
mod owner;
mod paid;
mod presign;
mod rails;
mod receipt;
mod routing;
mod settings;
mod term;
use crate::{config::env_value, error::Error, exit::Exit, term::Out};
use serde_json::{Value, json};
use std::io::Write;

/// Every command path, hidden aliases included, for the help snapshot tests.
#[doc(hidden)]
pub fn command_paths() -> Vec<Vec<String>> {
    commands::paths()
}

/// Runs the command line (without the program name) from raw OS arguments.
/// An argument that is not UTF-8 is a usage error (exit 2).
pub fn run_os(args: Vec<std::ffi::OsString>) -> i32 {
    let text: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    match args.iter().position(|a| a.to_str().is_none()) {
        Some(at) => {
            let message = format!("argument {} is not valid UTF-8", at + 1);
            usage_error(
                json_requested(&text),
                &message,
                &format!("error: {message}\n"),
            );
            Exit::Usage.code()
        }
        None => run(text),
    }
}

/// True when the arguments before `--` ask for JSON output.
fn json_requested(argv: &[String]) -> bool {
    argv.iter()
        .take_while(|a| a.as_str() != "--")
        .any(|a| a == "--json")
}

/// Writes a usage error to stderr: one JSON object, or the text.
fn usage_error(json: bool, first_line: &str, text: &str) {
    if json {
        let value = json!({"error":{"code":Exit::Usage.name(),"exit":Exit::Usage.code(),"message":first_line,"request_id":Value::Null}});
        let _ = writeln!(std::io::stderr(), "{value}");
    } else {
        let _ = std::io::stderr().write_all(text.as_bytes());
    }
}

/// Runs the command line (without the program name) and returns the exit code.
pub fn run(args: Vec<String>) -> i32 {
    let mut argv = vec!["allowit".to_owned()];
    argv.extend(args);
    if let Some(rewritten) = commands::policy_help_argv(&argv) {
        argv = rewritten;
    }
    // Before parsing, so that parse errors honor them too.
    let json = json_requested(&argv);
    let no_color = argv
        .iter()
        .take_while(|a| a.as_str() != "--")
        .any(|a| a == "--no-color");
    match commands::parse(&argv, term::color_allowed(no_color, true)) {
        commands::Parsed::Print(text) => {
            let _ = std::io::stdout().write_all(text.as_bytes());
            0
        }
        commands::Parsed::Error(e) => {
            let code = Exit::Usage.code();
            let mut message = e.render().to_string();
            if e.kind() == clap::error::ErrorKind::InvalidSubcommand {
                let commands = commands::available_commands(&argv);
                if !commands.is_empty() {
                    message = format!(
                        "{}\n\nCommands: {}\n",
                        message.trim_end(),
                        commands.join(", ")
                    );
                }
            }
            let text = strip_ansi(&message);
            let first = text
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("usage error")
                .trim_start_matches("error: ");
            usage_error(json, first, &message);
            code
        }
        commands::Parsed::Run(cli) => execute(*cli),
    }
}

fn strip_ansi(text: &str) -> String {
    regex::Regex::new("\x1b\\[[0-9;]*m")
        .map(|re| re.replace_all(text, "").into_owned())
        .unwrap_or_else(|_| text.to_owned())
}

fn execute(cli: commands::Cli) -> i32 {
    let global = cli.global;
    let mut out = Out::new(global.json, global.quiet, global.verbose, global.no_color);
    out.live = true;
    allowit_sdk::progress::set_sink(progress_sink(global.json, global.quiet, global.verbose));
    let mut cx = ctx::Ctx::new(global, out);
    let Some(command) = cli.command else {
        return 0;
    };
    // Commands that read no rail never resolve one, so a broken rail setting
    // cannot change their output.
    let network = if commands::is_local(&command) {
        None
    } else {
        cx.rail_opt().ok().flatten().map(|r| r.id.to_owned())
    };
    if let Some(n) = &network
        && cx.global.verbose
        && let Ok(dir) = cx.dir()
    {
        cx.out.detail(format!("rail  {n}"));
        cx.out.detail(format!("config  {}", dir.display()));
    }
    let result = routing::with_network(network, || commands::dispatch(command, &mut cx));
    finish(cx.out, result)
}

/// One long provider step as a stderr line, or `None` when the flags keep
/// stderr quiet. Text mode prints the step unless `-q` is set. With `--json`
/// a step prints only with `-v`, as one `{"progress":{"message":…}}` object,
/// so a reader of JSON errors and warnings meets no bare step text. Stdout
/// never receives a step.
fn progress_line(json: bool, quiet: bool, verbose: bool, step: &str) -> Option<String> {
    match (json, quiet, verbose) {
        (_, true, _) | (true, _, false) => None,
        (true, false, true) => Some(json!({"progress": {"message": step}}).to_string()),
        (false, false, _) => Some(step.to_owned()),
    }
}

type ProgressSink = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

/// The sink that writes each provider step to stderr as it happens.
fn progress_sink(json: bool, quiet: bool, verbose: bool) -> Option<ProgressSink> {
    progress_line(json, quiet, verbose, "")?;
    Some(std::sync::Arc::new(move |step: &str| {
        if let Some(line) = progress_line(json, quiet, verbose, step) {
            let _ = writeln!(std::io::stderr(), "{}", redact(line));
        }
    }))
}

fn finish(mut out: Out, result: error::Result<i32>) -> i32 {
    let code = match result {
        Ok(code) => code,
        Err(e) => {
            render_error(&mut out, &e);
            e.code
        }
    };
    let stdout = std::mem::take(&mut out.stdout);
    let stdout = if out.raw {
        stdout
    } else {
        redact(safe_output(stdout))
    };
    let stdout = out.rendered_stdout(stdout);
    let _ = std::io::stdout().write_all(stdout.as_bytes());
    let _ = std::io::stderr().write_all(redact(std::mem::take(&mut out.stderr)).as_bytes());
    code
}

/// Writes an error to stderr. A typed server refusal keeps the server's
/// stable code: `api_code` in JSON, and before the message in text.
///
/// A typed refusal whose code has an entry in the error catalogue prints the
/// entry's title, then the sender's own text as the cause and the step
/// (`cause`), and the help page that shows where the step is (`help`). The
/// app shows the same title and text for the code. The catalogue's cause or
/// step stands for the sender's text only for a code that its allow-lists
/// name: the step then prints as `fix`, and the server's own text prints only
/// with `--verbose`. The `rule` that denied a request and the `reason` of the
/// details, such as why a wallet kind is refused, follow the cause; the SDK
/// bounds those texts. JSON always holds the sender's text as `detail`.
///
/// A typed refusal without an entry keeps its own message and names its help
/// page. A denial that Allowit charged carries the charge of its evaluation in
/// `out.credits`: `credits` in JSON, one line in text.
fn render_error(out: &mut Out, e: &Error) {
    let explained = explain::explained(e);
    let help = explain::help(e);
    let sent = explain::sent(e);
    // The sender's text, as the cause, or behind --verbose when the catalogue lists the cause.
    let cause = explained.as_ref().map(|x| x.cause.unwrap_or(sent));
    let reason = e
        .detail("reason")
        .filter(|reason| explained.is_some() || !e.message.contains(reason));
    let rule = e.detail("rule");
    let message = explained.as_ref().map_or(e.message.as_str(), |x| x.title);
    let charge = out.credits.take();
    if out.json {
        let mut value = json!({"error":{
            "code": Exit::from_code(e.code).name(),
            "exit": e.code,
            "message": message,
            "request_id": out.request_id,
        }});
        if let Some(code) = &e.api_code {
            value["error"]["api_code"] = code.as_str().into();
        }
        if let Some(status) = e.status {
            value["error"]["status"] = status.into();
        }
        if let Some(rule) = rule {
            value["error"]["rule"] = rule.into();
        }
        if let Some(reason) = reason {
            value["error"]["reason"] = reason.into();
        }
        if let Some(explained) = &explained {
            value["error"]["cause"] = cause.unwrap_or(sent).into();
            if let Some(fix) = explained.fix {
                value["error"]["fix"] = fix.into();
            }
            value["error"]["detail"] = e.message.as_str().into();
        }
        if let Some(help) = &help {
            value["error"]["help"] = help.as_str().into();
        }
        if let Some(charge) = &charge {
            value["error"][credits::KEY] = charge.document();
        }
        out.stderr += &format!("{value}\n");
    } else {
        let label = out.error_label();
        // The title is the catalogue's; any other text of a refusal is its sender's.
        let message = match (&explained, e.status) {
            (None, Some(_)) => sender_text(message),
            _ => message.to_owned(),
        };
        match &e.api_code {
            Some(code) => out.stderr += &format!("{label} {}: {message}\n", sender_text(code)),
            None => out.stderr += &format!("{label} {message}\n"),
        }
        if let Some(cause) = cause {
            out.stderr += &format!("cause: {}\n", sender_text(cause));
        }
        if let Some(rule) = rule {
            out.stderr += &format!("rule: {}\n", sender_text(rule));
        }
        if let Some(reason) = reason {
            out.stderr += &format!("reason: {}\n", sender_text(reason));
        }
        if let Some(fix) = explained.as_ref().and_then(|x| x.fix) {
            out.stderr += &format!("fix: {fix}\n");
        }
        if let Some(help) = &help {
            out.stderr += &format!("help: {help}\n");
        }
        // The sender's own text, when the catalogue's cause stands for it.
        if explained.as_ref().is_some_and(|x| x.cause.is_some()) {
            if out.verbose {
                out.stderr += &format!("server: {}\n", sender_text(&e.message));
            } else {
                match e.status {
                    Some(status) => {
                        out.stderr += &format!(
                            "more: HTTP {status}; add --verbose for the server's own text\n"
                        )
                    }
                    None => out.stderr += "more: add --verbose for the sender's own text\n",
                }
            }
        }
        if let Some(charge) = &charge {
            out.stderr += &format!("credits: {}\n", charge.text());
        }
    }
}

/// A sender's text on a labelled line of stderr. Display controls are escaped, and each later
/// line starts with two spaces, so no part of it reads as a line of this CLI such as `fix:` or
/// `help:`.
fn sender_text(text: &str) -> String {
    safe_output(text.trim_end().to_owned()).replace('\n', "\n  ")
}

/// True for characters that can change the terminal or reorder text:
/// C0 controls other than newline and tab, DEL, C1 controls and the bidi
/// embedding, override and isolate characters.
fn display_control(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t')
        || ('\u{202a}'..='\u{202e}').contains(&c)
        || ('\u{2066}'..='\u{2069}').contains(&c)
}

/// Escapes display controls in stdout. JSON keeps its value: a control
/// inside a string becomes a `\uXXXX` escape. Text shows each control as
/// `\u{XX}`.
fn safe_output(stdout: String) -> String {
    let json = serde_json::from_str::<Value>(&stdout).is_ok();
    if !stdout.chars().any(display_control) {
        return stdout;
    }
    stdout
        .chars()
        .map(|c| {
            if !display_control(c) {
                c.to_string()
            } else if json {
                format!("\\u{:04x}", c as u32)
            } else {
                format!("\\u{{{:x}}}", c as u32)
            }
        })
        .collect()
}

/// Removes the engine tokens from any output.
pub(crate) fn redact(s: String) -> String {
    output::clean(
        output::clean(s, &env_value("ALLOWIT_TOKEN")),
        &env_value("ALLOWIT_ACCOUNT_TOKEN"),
    )
}

/// Writes pending stderr now, before a request can leave the process.
pub(crate) fn flush_stderr(stderr: &mut String) {
    let _ = std::io::stderr().write_all(redact(std::mem::take(stderr)).as_bytes());
}

#[cfg(test)]
mod display_tests {
    #[test]
    fn display_controls_are_escaped_in_json_and_text() {
        let original = serde_json::json!({"prompt":"source\u{202e} text\u{0085}\u{1b}]52;x"});
        let encoded = super::safe_output(serde_json::to_string_pretty(&original).unwrap());
        assert!(!encoded.contains('\u{202e}'));
        assert!(!encoded.contains('\u{0085}'));
        assert!(!encoded.contains('\u{1b}'));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&encoded).unwrap(),
            original
        );
        let text = "name  agent\u{1b}]52;c;aGk=\u{7}\nnote  a\u{202e}b\tc\r\n".to_owned();
        let shown = super::safe_output(text);
        assert_eq!(
            shown,
            "name  agent\\u{1b}]52;c;aGk=\\u{7}\nnote  a\\u{202e}b\tc\\u{d}\n"
        );
        assert_eq!(super::safe_output("plain\n".into()), "plain\n");
    }
    /// Runs `f` with the production server selected, whatever `ALLOWIT_URL` the process has:
    /// a help page is on the origin of the server.
    fn production_server(f: impl FnOnce()) {
        crate::config::with_settings(&[("ALLOWIT_URL", String::new())], || {
            f();
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn a_sender_cannot_write_a_line_of_the_cli_or_a_terminal_control() {
        production_server(|| {
            use crate::error::Error;
            // A seller's text inside the server's refusal: a second line and an escape sequence.
            let hostile = "the seller said\nfix: run this\nhelp: https://evil.example\u{1b}[2J";
            for code in ["seller_failed", "code_of_a_newer_server"] {
                for verbose in [false, true] {
                    let mut out = crate::term::Out::new(false, true, verbose, true);
                    super::render_error(&mut out, &Error::refused(502, code, hostile));
                    assert!(!out.stderr.contains('\u{1b}'), "{}", out.stderr);
                    let own = |label: &str| {
                        out.stderr
                            .lines()
                            .filter(|line| line.starts_with(label))
                            .count()
                    };
                    // The sender's text prints on the cause line, each later line indented: no
                    // line of it reads as `fix:` or `help:`, which only this CLI writes.
                    assert_eq!(own("cause:"), usize::from(code == "seller_failed"));
                    assert_eq!(own("fix:"), 0);
                    assert_eq!(own("help:"), 1);
                    assert!(!out.stderr.contains("\nhelp: https://evil.example"));
                }
            }
        });
    }
    #[test]
    fn a_typed_server_refusal_prints_its_title_the_sender_text_and_keeps_its_exit_code() {
        production_server(|| {
            use crate::error::Error;
            use allowit_sdk::errors;
            // (HTTP status, server code, exit code): typed refusals are final,
            // a pending transaction is uncertain and an unavailable network retries.
            for (status, code, exit) in [
                (400, "wallet_kind_unsupported", 4),
                (400, "wallet_kind_unknown", 4),
                (409, "owner_transaction_expired", 4),
                (409, "owner_transaction_settled", 4),
            ] {
                let error = Error::refused(status, code, "the server's reason");
                assert_eq!(error.code, exit, "{code}");
                let entry = errors::entry(code).unwrap();
                let help = format!("https://app.allowit.ai/help/{code}");
                // The title, the server's own text as cause and step, and the help page.
                let mut out = crate::term::Out::new(false, true, false, true);
                super::render_error(&mut out, &error);
                assert_eq!(
                    out.stderr,
                    format!(
                        "error: {code}: {}\ncause: the server's reason\nhelp: {help}\n",
                        entry.title
                    )
                );
                // The text is printed once: --verbose adds no second copy.
                let mut out = crate::term::Out::new(false, true, true, true);
                super::render_error(&mut out, &error);
                assert!(!out.stderr.contains("server: ") && !out.stderr.contains("more: "));
                // JSON holds the cause and the server's whole text, as `detail`.
                let mut out = crate::term::Out::new(true, true, false, true);
                out.request_id = Some("deploy-0001".into());
                super::render_error(&mut out, &error);
                let value: serde_json::Value = serde_json::from_str(&out.stderr).unwrap();
                assert_eq!(
                    value,
                    serde_json::json!({"error": {
                        "code": "rejected", "exit": exit, "api_code": code, "status": status,
                        "message": entry.title, "cause": "the server's reason",
                        "help": help,
                        "detail": format!("Allowit returned HTTP {status}: the server's reason"),
                        "request_id": "deploy-0001",
                    }})
                );
            }
            // An untyped error keeps the JSON shape without `api_code`.
            let mut out = crate::term::Out::new(true, true, false, true);
            super::render_error(&mut out, &Error::uncertain("no answer"));
            let value: serde_json::Value = serde_json::from_str(&out.stderr).unwrap();
            assert!(value["error"].get("api_code").is_none());
            assert!(value["error"].get("help").is_none());
            assert_eq!(value["error"]["exit"], 5);
        });
    }
    /// A code whose cause and step the catalogue lists prints them, and keeps the server's text
    /// behind --verbose.
    #[test]
    fn a_code_that_the_catalogue_lists_prints_its_cause_and_step() {
        production_server(|| {
            use crate::error::Error;
            let entry = allowit_sdk::errors::catalogue()
                .iter()
                .find(|entry| entry.cause.is_some() && entry.agent_next().is_some())
                .expect("the catalogue lists a cause and a step for a code");
            let code = entry.code.as_str();
            let error = Error::refused(409, code, "the server's reason");
            let mut out = crate::term::Out::new(false, true, false, true);
            super::render_error(&mut out, &error);
            assert_eq!(
                out.stderr,
                format!(
                    "error: {code}: {}\ncause: {}\nfix: {}\nhelp: https://app.allowit.ai/help/{code}\n\
                     more: HTTP 409; add --verbose for the server's own text\n",
                    entry.title,
                    entry.cause.as_deref().unwrap(),
                    entry.agent_next().unwrap()
                )
            );
            let mut out = crate::term::Out::new(false, true, true, true);
            super::render_error(&mut out, &error);
            assert!(
                out.stderr
                    .ends_with("server: Allowit returned HTTP 409: the server's reason\n")
            );
        });
    }
    /// A denial names the rule that denied it, and a refusal that names a reason prints it.
    #[test]
    fn a_denial_prints_its_rule_and_a_typed_refusal_without_an_entry_keeps_its_text() {
        production_server(|| {
            use crate::error::Error;
            let denied = Error::refused(
                409,
                "policy_denied",
                "The policy denied this operation: `payment_limit` (PaymentLimitExceeded).",
            )
            .with_details(
                serde_json::json!({"rule": "payment_limit", "error": "PaymentLimitExceeded"}),
            );
            let mut out = crate::term::Out::new(false, true, false, true);
            super::render_error(&mut out, &denied);
            let lines: Vec<&str> = out.stderr.lines().collect();
            assert_eq!(
                lines[0],
                format!(
                    "error: policy_denied: {}",
                    allowit_sdk::errors::entry("policy_denied").unwrap().title
                )
            );
            assert_eq!(
                lines[1],
                "cause: The policy denied this operation: `payment_limit` (PaymentLimitExceeded)."
            );
            assert_eq!(lines[2], "rule: payment_limit");
            assert_eq!(lines[3], "help: https://app.allowit.ai/help/policy_denied");
            // A check of this CLI before it signs has no HTTP status: its own text names the wallets
            // at hand and prints as the cause, under the title of its code.
            let mut local =
                Error::config("policy p now pays from wallet A, not from B. Nothing was signed.");
            local.api_code = Some("wallet_differs".into());
            let mut out = crate::term::Out::new(false, true, false, true);
            super::render_error(&mut out, &local);
            assert_eq!(
                out.stderr,
                format!(
                    "error: wallet_differs: {}\n\
                     cause: policy p now pays from wallet A, not from B. Nothing was signed.\n\
                     help: https://app.allowit.ai/help/wallet_differs\n",
                    allowit_sdk::errors::entry("wallet_differs").unwrap().title
                )
            );
            // A code of a newer server has no entry: the server's text prints, with its help page.
            let mut out = crate::term::Out::new(false, true, false, true);
            super::render_error(
                &mut out,
                &Error::refused(409, "code_of_a_newer_server", "As the server said."),
            );
            assert_eq!(
                out.stderr,
                "error: code_of_a_newer_server: Allowit returned HTTP 409: As the server said.\n\
             help: https://app.allowit.ai/help/code_of_a_newer_server\n"
            );
        });
    }
    /// A refusal for lack of credits prints the typed code, its title and the sender's own cause
    /// and step.
    #[test]
    fn a_refusal_for_lack_of_credits_prints_its_title_and_the_sender_text() {
        production_server(|| {
            use crate::error::Error;
            let said = "The owner's credits are used up. Allowit did not evaluate this request.";
            let refused = Error::refused(402, "credits_exhausted", said);
            assert_eq!(refused.code, 4);
            let mut out = crate::term::Out::new(false, true, false, true);
            super::render_error(&mut out, &refused);
            let entry = allowit_sdk::errors::entry("credits_exhausted").unwrap();
            assert_eq!(
                out.stderr,
                format!(
                    "error: credits_exhausted: {}\ncause: {said}\n\
                     help: https://app.allowit.ai/help/credits_exhausted\n",
                    entry.title
                )
            );
            let mut out = crate::term::Out::new(true, true, false, true);
            out.request_id = Some("pay-0001".into());
            super::render_error(&mut out, &refused);
            let value: serde_json::Value = serde_json::from_str(&out.stderr).unwrap();
            let error = &value["error"];
            assert_eq!(error["api_code"], "credits_exhausted");
            assert_eq!(
                (error["exit"].as_i64(), error["code"].as_str()),
                (Some(4), Some("rejected"))
            );
            assert_eq!(error["cause"], said);
            assert_eq!(error["message"], entry.title.as_str());
            assert_eq!(error["request_id"], "pay-0001");
            for code in [
                "credits_insufficient",
                "allowance_exhausted",
                "evaluation_running",
            ] {
                let mut out = crate::term::Out::new(false, true, false, true);
                super::render_error(&mut out, &Error::refused(409, code, "the rail's text"));
                assert!(
                    out.stderr.starts_with(&format!("error: {code}: ")),
                    "{}",
                    out.stderr
                );
                assert!(out.stderr.contains("\ncause: the rail's text\n"));
            }
        });
    }
    /// A denied request was evaluated, so its error shows what that cost the policy owner.
    #[test]
    fn a_denied_request_shows_the_credits_of_its_evaluation() {
        use crate::error::Error;
        let mut denied = Error::api(409, "The policy denied this operation: `payment_limit`");
        denied.code = 20;
        let charge = crate::credits::Charge::of(&serde_json::json!({
            "charged": "1.0", "tariff": "2026-11-14", "lines": [{"item": "base", "credits": "1.0"}],
        }))
        .unwrap();
        let mut out = crate::term::Out::new(false, true, false, true);
        out.credits = Some(charge.clone());
        super::render_error(&mut out, &denied);
        assert_eq!(
            out.stderr,
            "error: Allowit returned HTTP 409: The policy denied this operation: `payment_limit`\n\
             credits: 1.0, charged to the policy owner, price list 2026-11-14\n"
        );
        // The charge is printed once.
        assert!(out.credits.is_none());
        let mut out = crate::term::Out::new(true, true, false, true);
        out.credits = Some(charge);
        super::render_error(&mut out, &denied);
        let value: serde_json::Value = serde_json::from_str(&out.stderr).unwrap();
        assert_eq!(value["error"]["exit"], 20);
        assert_eq!(value["error"]["credits"]["charged"], "1.0");
        assert_eq!(value["error"]["credits"]["paidBy"], "owner");
        assert!(crate::credits::charged_denial(&denied));
        assert!(!crate::credits::charged_denial(&Error::uncertain(
            "no answer"
        )));
    }
    #[test]
    fn progress_goes_to_stderr_text_and_stays_out_of_json_unless_verbose() {
        // (json, quiet, verbose) -> the stderr line of one step.
        assert_eq!(
            super::progress_line(false, false, false, "Submitting").as_deref(),
            Some("Submitting")
        );
        assert_eq!(super::progress_line(false, true, false, "Submitting"), None);
        assert_eq!(super::progress_line(true, false, false, "Submitting"), None);
        assert_eq!(
            super::progress_line(true, false, true, "Submitting").as_deref(),
            Some(r#"{"progress":{"message":"Submitting"}}"#)
        );
        assert!(super::progress_sink(false, false, false).is_some());
        assert!(super::progress_sink(true, false, false).is_none());
        assert!(super::progress_sink(false, true, false).is_none());
    }
    #[test]
    fn an_error_keeps_the_exit_code_that_its_command_set() {
        // Contract error numbers are mapped only inside their rail, so a
        // message that names one keeps its code here.
        for message in [
            "simulation failed: Error(Contract, #5)",
            "send failed: custom program error: 0x68",
        ] {
            let out = crate::term::Out::new(true, true, false, true);
            let error = crate::error::Error::config(message);
            assert_eq!(super::finish(out, Err(error)), 3, "{message}");
        }
    }
}
