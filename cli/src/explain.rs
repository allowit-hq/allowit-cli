//! What a typed refusal means: the text of the error catalogue for its code, and its help page.
//!
//! The catalogue is Core's (`allowit_sdk::errors`). The app shows the same cause and next step
//! for a code; the page `<ALLOWIT_URL origin>/help/<code>` shows the fix. The server's own text
//! of a refusal prints only with `--verbose`, and JSON output always holds it as `detail`.
use crate::error::Error;
use allowit_sdk::errors;

/// The catalogue's text for a refusal of the server.
pub(crate) struct Explained {
    /// One line that names the problem.
    pub title: &'static str,
    pub cause: &'static str,
    /// What the reader of a terminal does next.
    pub fix: &'static str,
}

/// The explanation of a typed refusal that the server sent. A check of this CLI before it signs
/// has no HTTP status: its own message names the keys and wallets at hand, so it stays as it is.
pub(crate) fn explained(error: &Error) -> Option<Explained> {
    error.status?;
    let entry = errors::entry(error.api_code.as_deref()?)?;
    Some(Explained {
        title: &entry.title,
        cause: &entry.cause,
        fix: entry.agent_next(),
    })
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
    Some(errors::help_url(&origin, code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_refusal_with_a_catalogue_code_has_a_cause_a_fix_and_a_help_page() {
        let denied = Error::refused(409, "policy_denied", "The policy denied this operation.");
        let text = explained(&denied).unwrap();
        let entry = errors::entry("policy_denied").unwrap();
        assert_eq!(text.title, entry.title);
        assert_eq!(text.cause, entry.cause);
        // The terminal reader is the agent: it gets the agent's next step.
        assert_eq!(text.fix, entry.agent.as_deref().unwrap());
        assert!(help(&denied).unwrap().ends_with("/help/policy_denied"));
    }

    #[test]
    fn each_credit_refusal_has_a_cause_and_a_fix_for_the_owner() {
        for code in [
            "credits_exhausted",
            "credits_insufficient",
            "allowance_exhausted",
        ] {
            let text = explained(&Error::refused(402, code, "x")).unwrap();
            assert!(text.fix.contains("owner"), "{code}");
            assert!(!text.cause.is_empty() && !text.title.is_empty());
        }
        // The agent never needs credits of its own, and a request that was not evaluated keeps
        // its request ID.
        let exhausted = explained(&Error::refused(402, "credits_exhausted", "x")).unwrap();
        assert!(exhausted.fix.contains("You need no credits of your own"));
        assert!(exhausted.fix.contains("same request ID"));
        assert!(
            exhausted
                .cause
                .contains("nothing was signed, sent or charged")
        );
        let running = explained(&Error::refused(409, "evaluation_running", "x")).unwrap();
        assert!(running.fix.contains("allowit status"));
        let unavailable = explained(&Error::refused(503, "credits_unavailable", "x")).unwrap();
        assert!(unavailable.fix.contains("same request ID"));
    }

    #[test]
    fn a_local_check_and_an_unknown_code_keep_their_own_text() {
        // A check of this CLI before it signs: no HTTP status.
        let mut local = Error::config("policy p now pays from wallet A, not from B.");
        local.api_code = Some("wallet_differs".into());
        assert!(explained(&local).is_none());
        assert!(help(&local).unwrap().ends_with("/help/wallet_differs"));
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
    }
}
