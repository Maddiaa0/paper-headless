//! Relayed sign-in. Paper signs in through the system browser (WorkOS PKCE)
//! and expects a `paper://auth/callback?code=…` deep link back. On a server
//! there is no browser, so: click "Sign in" over DevTools, capture the URL the
//! app tried to open, let the user finish in a browser anywhere, and feed the
//! resulting code back through a second Paper instance.

use crate::settings::Settings;
use crate::{Error, Result, cdp, mcp, paper, paths};
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::thread::sleep;
use std::time::{Duration, Instant};

const SIGN_IN_PAGE: &str = "/desktop/sign-in";
/// Chromium persists cookies to disk on a timer; restarting sooner loses them.
const COOKIE_FLUSH_WAIT: Duration = Duration::from_secs(35);

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum SignInState {
    SignedOut,
    SignedIn,
    Unknown,
}

pub(crate) fn sign_in_state(settings: &Settings) -> Result<SignInState> {
    let pages = cdp::pages(&settings.cdp_url())?;
    state_from_pages(&pages)
}

fn state_from_pages(pages: &[cdp::Target]) -> Result<SignInState> {
    let paths: Vec<_> = pages
        .iter()
        .filter_map(|page| page.url.strip_prefix("https://app.paper.design/"))
        .filter_map(|path| path.split(['?', '#']).next())
        .collect();
    if paths
        .iter()
        .any(|path| matches!(*path, "error" | "error.html"))
    {
        return Err(Error::msg(
            "Paper rejected sign-in. Run `paper-headless restart`, then `paper-headless login` \
             for a fresh callback. Do not let a local Paper app consume the callback first.",
        ));
    }
    if paths.iter().any(|path| path.contains(SIGN_IN_PAGE)) {
        return Ok(SignInState::SignedOut);
    }
    if paths
        .iter()
        .any(|path| !path.starts_with("static/desktop/") && !path.starts_with("www/desktop/"))
    {
        return Ok(SignInState::SignedIn);
    }
    Ok(SignInState::Unknown)
}

fn require_running(settings: &Settings) -> Result<()> {
    cdp::pages(&settings.cdp_url()).map(|_| ()).map_err(|_| {
        Error::msg(format!(
            "Paper is not reachable on {}; start it with `paper-headless start` \
             (or `paper-headless serve` in a terminal) and try again",
            settings.cdp_url()
        ))
    })
}

pub(crate) fn login(settings: &Settings, wait: bool) -> Result<()> {
    require_running(settings)?;
    match sign_in_state(settings)? {
        SignInState::SignedIn => {
            println!("Paper is already signed in.");
            println!("{}", mcp::describe(&mcp::probe(&settings.mcp_url())?));
            return Ok(());
        }
        SignInState::Unknown => {
            return Err(Error::msg(
                "could not find Paper's sign-in page; wait a few seconds after start and retry, \
                 or check `paper-headless logs`",
            ));
        }
        SignInState::SignedOut => {}
    }

    let auth_url_file = settings.auth_url_file();
    fs::write(&auth_url_file, b"")?;
    let pages = cdp::pages(&settings.cdp_url())?;
    let page = pages
        .iter()
        .find(|page| page.url.contains(SIGN_IN_PAGE))
        .and_then(|page| page.ws_url.as_deref())
        .ok_or_else(|| Error::msg("sign-in page has no DevTools socket"))?;
    cdp::Session::connect(page)?.click_first_button()?;

    let url = wait_for_line(&auth_url_file, Duration::from_secs(20)).ok_or_else(|| {
        Error::msg(format!(
            "Paper did not ask to open a browser within 20s; see {}",
            settings.paper_log().display()
        ))
    })?;

    println!();
    println!("Open this URL in a browser on any machine and sign in to Paper:");
    println!();
    println!("  {url}");
    println!();
    println!("You will land on an \"Opening Paper…\" page at workers.paper.design.");
    println!("If a local Paper app opens, close it. Then copy the link behind the");
    println!("\"click here\" fallback (it starts with paper://auth/callback?code=)");
    println!("or copy the page's address bar; either one works.");
    println!();

    if !wait {
        println!("Finish with:  paper-headless login-code '<that link>'");
        return Ok(());
    }
    if !io::stdin().is_terminal() {
        println!("stdin is not a terminal; finish with:  paper-headless login-code '<that link>'");
        return Ok(());
    }
    print!("Paste the link or code here: ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    if line.trim().is_empty() {
        println!("Nothing pasted; finish later with:  paper-headless login-code '<that link>'");
        return Ok(());
    }
    login_code(settings, line.trim(), true)
}

pub(crate) fn login_code(settings: &Settings, input: &str, wait_for_flush: bool) -> Result<()> {
    let code = extract_code(input)?;
    require_running(settings)?;
    if sign_in_state(settings)? == SignInState::SignedIn {
        println!("Paper is already signed in.");
        return Ok(());
    }
    println!("Delivering the sign-in code to the running Paper instance…");
    paper::send_deep_link(settings, &format!("paper://auth/callback?code={code}"))?;

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if sign_in_state(settings)? == SignInState::SignedIn {
            break;
        }
        sleep(Duration::from_secs(1));
    }
    if sign_in_state(settings)? != SignInState::SignedIn {
        return Err(Error::msg(format!(
            "Paper is still on its sign-in page. Codes are single-use and expire quickly; \
             run `paper-headless login` again for a fresh URL. Recent log lines:\n{}",
            paths::tail(&settings.paper_log(), 8)
        )));
    }
    println!("Signed in.");

    let deadline = Instant::now() + Duration::from_secs(60);
    let mut last = None;
    while Instant::now() < deadline {
        let probe = mcp::probe(&settings.mcp_url())?;
        if probe.is_ready() {
            println!("{}", mcp::describe(&probe));
            last = None;
            break;
        }
        last = Some(mcp::describe(&probe));
        sleep(Duration::from_secs(2));
    }
    if let Some(detail) = last {
        println!("warning: {detail}");
    }

    if wait_for_flush {
        print!(
            "Waiting {}s so Chromium writes the session to disk before any restart…",
            COOKIE_FLUSH_WAIT.as_secs()
        );
        io::stdout().flush()?;
        sleep(COOKIE_FLUSH_WAIT);
        println!(" done.");
    }
    println!(
        "Login persists in {} across restarts.",
        settings.profile_dir().display()
    );
    Ok(())
}

/// Accept a bare code, the `paper://auth/callback?code=…` link, or the
/// `https://workers.paper.design/auth/desktop-redirect?…&code=…` page URL.
pub(crate) fn extract_code(input: &str) -> Result<String> {
    let input = input.trim();
    if let Some(rest) = input.split("code=").nth(1) {
        let code: String = rest
            .chars()
            .take_while(|c| *c != '&' && *c != '#' && !c.is_whitespace())
            .collect();
        if !code.is_empty() {
            return Ok(code);
        }
    }
    if input.is_empty() || input.contains("://") || input.contains(char::is_whitespace) {
        return Err(Error::msg(
            "expected a sign-in code or a link containing code=…",
        ));
    }
    Ok(input.to_owned())
}

fn wait_for_line(path: &std::path::Path, timeout: Duration) -> Option<String> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if let Some(line) = fs::read_to_string(path).ok().and_then(|contents| {
            contents
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(str::to_owned)
        }) {
            return Some(line);
        }
        sleep(Duration::from_millis(250));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_and_background_pages_are_not_signed_in() {
        let page = |url: &str| cdp::Target {
            kind: "page".into(),
            url: format!("https://app.paper.design/{url}"),
            ws_url: None,
        };
        let mut pages = vec![
            page("static/desktop/preloader"),
            page("www/desktop/app-bar"),
        ];
        assert_eq!(state_from_pages(&pages).unwrap(), SignInState::Unknown);
        pages.push(page("error?message=Could%20not%20complete%20signing%20in"));
        assert!(state_from_pages(&pages).is_err());
        pages.pop();
        pages.push(page("www/desktop/sign-in"));
        assert_eq!(state_from_pages(&pages).unwrap(), SignInState::SignedOut);
        pages.pop();
        pages.push(page(""));
        assert_eq!(state_from_pages(&pages).unwrap(), SignInState::SignedIn);
    }

    mod when_reading_a_sign_in_code {
        use super::*;

        #[test]
        fn takes_a_bare_code() {
            assert_eq!(extract_code("ABC123").unwrap(), "ABC123");
        }

        #[test]
        fn takes_a_callback_link() {
            assert_eq!(
                extract_code("paper://auth/callback?code=ABC123").unwrap(),
                "ABC123"
            );
        }

        #[test]
        fn takes_a_redirect_url() {
            assert_eq!(
                extract_code(
                    "https://workers.paper.design/auth/desktop-redirect?protocol=paper&code=ABC123&state=xyz"
                )
                .unwrap(),
                "ABC123"
            );
        }

        #[test]
        fn rejects_a_link_with_no_code() {
            assert!(extract_code("https://example.com/").is_err());
        }

        #[test]
        fn rejects_empty_input() {
            assert!(extract_code("").is_err());
        }
    }
}
