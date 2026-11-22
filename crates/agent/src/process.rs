use crate::{Config,
            Provider,
            browser};
use anyhow::{Result,
             bail};
use std::process::Stdio;
use tokio::{io::{AsyncBufRead,
                 AsyncBufReadExt,
                 AsyncReadExt},
            process::Command};

pub(crate) const MAX_LINE: usize = 16 * 1024 * 1024;
pub(crate) const AUTH_PREFIX: &str = "Open the following link to authenticate the ACP server: ";

pub(crate) fn command(config: &Config) -> Command {
    let mut command = Command::new(&config.executable);
    command.current_dir(&config.cwd).kill_on_drop(true);
    command.env_remove(crate::mcp::TOKEN).env_remove(crate::mcp::URL);
    #[cfg(unix)]
    command.process_group(0);
    // Console-subsystem runtimes (node/python shims) would flash a console
    // window.
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let removed: &[&str] = match config.provider {
        | Provider::Claude => &[
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_USE_FOUNDRY",
        ],
        | Provider::Codex => &["OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_ACCESS_TOKEN", "OPENAI_BASE_URL"],
        | Provider::Antigravity => &[
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "GOOGLE_CLOUD_PROJECT",
            "GOOGLE_CLOUD_LOCATION",
            "GOOGLE_CLOUD_QUOTA_PROJECT",
            "GOOGLE_GENAI_USE_VERTEXAI",
            "GCLOUD_PROJECT",
            "CLOUDSDK_CORE_PROJECT",
            "AGY_ACP_CCPA_PROJECT",
            "AGY_ACP_ENABLE_OAUTH",
            "ANTIGRAVITY_HARNESS_PATH",
        ],
    };
    // Environment variable names are case insensitive on Windows.
    for (key, _) in std::env::vars_os() {
        if removed.iter().any(|entry| key.to_string_lossy().eq_ignore_ascii_case(entry)) {
            command.env_remove(key);
        }
    }
    match config.provider {
        | Provider::Claude => {
            command
                .env("CLAUDE_CONFIG_DIR", &config.home)
                .env("DISABLE_AUTOUPDATER", "1")
                .env("DISABLE_UPDATES", "1");
        },
        | Provider::Codex => {
            command.env("CODEX_HOME", &config.home);
        },
        | Provider::Antigravity => {
            command
                .env("GEMINI_HOME", &config.home)
                .env("AGY_ACP_FORCE_FILE_STORAGE", "1")
                .env("PYTHONUNBUFFERED", "1");
        },
    }
    command
}

pub use gpui_agent_process::{Process,
                             spawn};

pub(crate) async fn line(reader: &mut (impl AsyncBufRead + Unpin)) -> Result<Option<String>> {
    let mut bytes = Vec::new();
    let count = reader.take((MAX_LINE + 1) as u64).read_until(b'\n', &mut bytes).await?;
    if count == 0 {
        return Ok(None);
    }
    if count > MAX_LINE {
        bail!("native runtime line exceeds 16 MiB");
    }
    Ok(Some(String::from_utf8(bytes)?.trim_end_matches(['\r', '\n']).to_owned()))
}

pub(crate) fn authorization(line: &str) -> Result<Option<String>> {
    if let Some(url) = line.strip_prefix(AUTH_PREFIX) {
        browser::validate(url, true)?;
        Ok(Some(url.to_owned()))
    } else if let Some(encoded) = line.strip_prefix(browser::MARKER) {
        let url: String = serde_json::from_str(encoded)?;
        browser::validate(&url, true)?;
        Ok(Some(url))
    } else {
        Ok(None)
    }
}

pub(crate) async fn inherited(config: &Config, args: &[&str]) -> Result<()> {
    let status = spawn(
        command(config)
            .args(args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit()),
    )?
    .wait()
    .await?;
    if !status.success() {
        bail!("native authentication command failed ({status})");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hosted_claude_cannot_replace_managed_or_path_installations() {
        let config = Config {
            provider: Provider::Claude,
            executable: "not-executed".into(),
            home: "profile".into(),
            cwd: ".".into(),
            harness: None,
            browser_helper: None,
            timeout: std::time::Duration::from_secs(1),
            mcp_home: None,
            mcp_selection: None,
            plugin_mcp: vec![],
        };
        let cmd = command(&config);
        for key in ["DISABLE_AUTOUPDATER", "DISABLE_UPDATES"] {
            assert!(
                cmd.as_std()
                    .get_envs()
                    .any(|(name, value)| name == key && value == Some(std::ffi::OsStr::new("1")))
            );
        }
    }
}
