use anyhow::{Result,
             ensure};
use std::path::Path;
use url::Url;

pub(crate) const MARKER: &str = "__T3_RUST_AUTH_URL__";

/// Call before parsing application arguments when using this executable as
/// Config.browser_helper.
pub fn run_browser_helper() -> bool {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("__auth-browser")) {
        return false;
    }
    if let Some(url) = args.next().and_then(|url| url.into_string().ok()) {
        match serde_json::to_string(&url) {
            | Ok(url) => eprintln!("{MARKER}{url}"),
            | Err(error) => eprintln!("Cannot encode authorization URL: {error}"),
        }
    }
    true
}

pub(crate) fn command(executable: &Path) -> Result<String> {
    let executable = executable.canonicalize()?;
    let executable = executable.to_str().ok_or_else(|| anyhow::anyhow!("browser helper path must be UTF-8"))?;
    // Python webbrowser splits on the platform path separator before shlex
    // processes quotes.
    let separator = if cfg!(windows) { ';' } else { ':' };
    ensure!(
        !executable.contains([separator, '\r', '\n', '\0']) && !executable.contains("%s"),
        "browser helper path cannot be represented in BROWSER"
    );
    let executable = if cfg!(windows) {
        executable.trim_start_matches(r"\\?\").replace('\\', "/")
    } else {
        executable.into()
    };
    Ok(format!("'{}' __auth-browser '%s'", executable.replace('\'', "'\"'\"'")))
}

pub(crate) fn validate(url: &str, google: bool) -> Result<()> {
    ensure!(url.len() <= 16_384 && !url.contains(['\r', '\n', '\0']), "invalid sign-in URL");
    let parsed = Url::parse(url)?;
    let hosts: &[&str] = if google {
        &["accounts.google.com"]
    } else {
        &["auth.openai.com", "chatgpt.com"]
    };
    ensure!(
        parsed.scheme() == "https"
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.port_or_known_default() == Some(443)
            && parsed.host_str().is_some_and(|host| hosts.contains(&host)),
        "unexpected sign-in URL origin"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_provider_https_origins_can_open() {
        assert!(validate("https://accounts.google.com/o/oauth2/v2/auth?state=test", true).is_ok());
        assert!(validate("https://auth.openai.com/authorize", false).is_ok());
        for url in [
            "file:///secret",
            "https://accounts.google.com.evil.test/",
            "https://evil@accounts.google.com/",
            "https://accounts.google.com:8443/",
        ] {
            assert!(validate(url, true).is_err(), "{url}");
        }
    }
}
