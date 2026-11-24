//! What a typed refusal means: the title of its code in the error catalogue, the sender's own
//! text as its cause and step, and its help page.
//!
//! The catalogue is Core's (`allowit_sdk::errors`), a registry: each code has a title, and a cause
//! or a step only when the catalogue's allow-lists name the code. Otherwise the sender's own text
//! states what happened and what to do. The app shows the same title and text for a code; the page
//! `<ALLOWIT_URL origin>/help/<code>` shows where the step is.
use crate::error::Error;
use allowit_sdk::errors;

/// The catalogue's text for a typed refusal: of the server, or of a check of this CLI before it
/// signs.
pub(crate) struct Explained {
    /// One line that names the code.
    pub title: &'static str,
    /// The catalogue's cause, for a code that its allow-list names. Otherwise the sender's text is
    /// the cause.
    pub cause: Option<&'static str>,
    /// The catalogue's step for the reader of a terminal, for a code that its allow-list names.
    pub fix: Option<&'static str>,
}

/// The explanation of a typed refusal whose code has an entry in the catalogue.
pub(crate) fn explained(error: &Error) -> Option<Explained> {
    let entry = errors::entry(error.api_code.as_deref()?)?;
    Some(Explained {
        title: &entry.title,
        cause: entry.cause.as_deref(),
        fix: entry.agent_next(),
    })
}

/// The sender's own text of a refusal: a server's text without the transport's
/// `Allowit returned HTTP <status>: ` prefix, which the status already states.
pub(crate) fn sent(error: &Error) -> &str {
    error
        .status
        .and_then(|status| {
            error
                .message
                .strip_prefix(&format!("Allowit returned HTTP {status}: "))
        })
        .unwrap_or(&error.message)
}

/// The help page of a typed refusal: `<origin>/help/<code>` on the origin of `ALLOWIT_URL`, which
/// serves the app. A code of a newer server has a page too. An invalid `ALLOWIT_URL` gives none.
pub(crate) fn help(error: &Error) -> Option<String> {
    let code = error
        .api_code
        .as_deref()
        .filter(|code| errors::is_code(code))?;
    let origin =
        crate::config::parse_origin(crate::config::env_value("ALLOWIT_URL").trim()).ok()?;
    // Core gives no address for an origin that a URL parser reads otherwise than it is written,
    // such as one with a tab or a line break in its authority: the link prints as it is.
    errors::help_url(&origin, code)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `f` with `https://allowit.example` as the server, whatever `ALLOWIT_URL` the process
    /// has: a help page is on the origin of the server.
    fn at_example(f: impl FnOnce()) {
        crate::config::with_settings(
            &[("ALLOWIT_URL", "https://allowit.example".to_owned())],
            || {
                f();
                Ok(())
            },
        )
        .unwrap();
    }

    #[test]
    fn a_typed_refusal_with_a_catalogue_code_has_its_title_and_a_help_page() {
        at_example(|| {
            let denied = Error::refused(409, "policy_denied", "The policy denied this operation.");
            let text = explained(&denied).unwrap();
            let entry = errors::entry("policy_denied").unwrap();
            assert_eq!(text.title, entry.title);
            // The registry lists no cause or step for the code: the sender's text states both.
            assert_eq!((text.cause, text.fix), (None, None));
            assert_eq!(sent(&denied), "The policy denied this operation.");
            assert_eq!(
                help(&denied).as_deref(),
                Some("https://allowit.example/help/policy_denied")
            );
            // A check of this CLI before it signs is explained the same way, with its own text.
            let mut local = Error::config("policy p now pays from wallet A, not from B.");
            local.api_code = Some("wallet_differs".into());
            assert_eq!(
                explained(&local).unwrap().title,
                errors::entry("wallet_differs").unwrap().title
            );
            assert_eq!(sent(&local), "policy p now pays from wallet A, not from B.");
        });
    }

    #[test]
    fn a_code_that_the_catalogue_lists_has_its_cause_and_step() {
        let listed = errors::catalogue()
            .iter()
            .find(|entry| entry.cause.is_some() && entry.agent_next().is_some())
            .expect("the catalogue lists a cause and a step for a code");
        let text = explained(&Error::refused(409, listed.code.as_str(), "x")).unwrap();
        assert_eq!(text.cause, listed.cause.as_deref());
        assert_eq!(text.fix, listed.agent_next());
    }

    #[test]
    fn an_unknown_code_keeps_its_own_text_and_has_a_help_page() {
        at_example(|| {
            // A newer server's code has no entry yet and still has a help page.
            let newer = Error::refused(409, "code_of_a_newer_server", "x");
            assert!(explained(&newer).is_none());
            assert!(
                help(&newer)
                    .unwrap()
                    .ends_with("/help/code_of_a_newer_server")
            );
            // Text that is no code, and an untyped error, have no page.
            assert!(help(&Error::refused(409, "Not A Code", "x")).is_none());
            assert!(help(&Error::api(500, "x")).is_none());
            assert!(explained(&Error::api(500, "x")).is_none());
        });
    }

    #[test]
    fn an_origin_with_a_line_break_gives_no_help_link() {
        for origin in [
            "https://app\n.example",
            "https://app\t.example",
            "https://app.ex\rample",
        ] {
            crate::config::with_settings(&[("ALLOWIT_URL", origin.to_owned())], || {
                let denied = Error::refused(409, "policy_denied", "x");
                assert_eq!(help(&denied), None, "{origin:?}");
                Ok(())
            })
            .unwrap();
        }
    }
}
