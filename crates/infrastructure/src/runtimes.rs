use crate::Settings;
use anyhow::{Context,
             Result,
             bail,
             ensure};
use gpui_agent_domain::{Provider,
                        RuntimeCommand,
                        RuntimeInfo,
                        RuntimeRelease,
                        provider_name,
                        valid_runtime_version as version_ok};
use serde_json::Value;
use sha2::{Digest,
           Sha256};
use std::{io::{Read,
               Write},
          path::{Path,
                 PathBuf},
          time::Duration};
mod antigravity;

const DOWNLOAD_LIMIT: u64 = 512 * 1024 * 1024;
fn binary(provider: Provider) -> &'static str {
    match provider {
        | Provider::Claude if cfg!(windows) => "claude.exe",
        | Provider::Codex if cfg!(windows) => "codex.exe",
        | _ => gpui_agent::default_executable(provider),
    }
}
fn root(settings: &Settings, provider: Provider) -> PathBuf { settings.runtime_root.join(provider_name(provider)) }
fn selected(settings: &Settings, provider: Provider) -> Result<Option<(String, String)>> {
    let file = root(settings, provider).join("current.json");
    if !file.exists() {
        return Ok(None);
    }
    let raw: Value = serde_json::from_reader(std::fs::File::open(file)?)?;
    let version = raw["version"].as_str().context("Invalid managed runtime pointer")?;
    let directory = raw["directory"].as_str().unwrap_or(version);
    ensure!(version_ok(version) && version_ok(directory), "Invalid managed runtime version");
    Ok(Some((version.into(), directory.into())))
}
pub(crate) fn managed_path(settings: &Settings, provider: Provider) -> Result<Option<PathBuf>> {
    let Some((_, directory)) = selected(settings, provider)? else {
        return Ok(None);
    };
    let file = root(settings, provider).join(directory).join(binary(provider));
    ensure!(file.is_file(), "Managed runtime is missing; reinstall it from the runtime panel");
    Ok(Some(file))
}
fn allowed(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "downloads.claude.ai"
                    | "api.github.com"
                    | "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
                    | "raw.githubusercontent.com"
                    | "dl.google.com"
            )
        )
}
fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent("gpui-agent-runtime-manager")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(420))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() < 5 && allowed(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("Untrusted runtime redirect")
            }
        }))
        .build()?)
}
fn get(client: &reqwest::blocking::Client, url: &str, limit: u64) -> Result<Vec<u8>> {
    ensure!(allowed(&url.parse()?), "Untrusted runtime URL");
    let response = client.get(url).send()?.error_for_status()?;
    ensure!(response.content_length().is_none_or(|len| len <= limit), "Runtime response is too large");
    let mut bytes = Vec::new();
    response.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "Runtime response is too large");
    Ok(bytes)
}
fn json(client: &reqwest::blocking::Client, url: &str) -> Result<Value> { Ok(serde_json::from_slice(&get(client, url, 4 * 1024 * 1024)?)?) }
fn platform() -> Result<(&'static str, &'static str)> {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        bail!("Runtime installation is unavailable on this OS")
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        bail!("Runtime installation requires x64 or arm64")
    };
    Ok((os, arch))
}
struct Release {
    public: RuntimeRelease,
    format: &'static str,
    source_binary: String,
}
fn release(client: &reqwest::blocking::Client, settings: &Settings, provider: Provider, requested: Option<&str>) -> Result<Release> {
    if let Some(version) = requested {
        ensure!(version_ok(version), "Invalid runtime version");
    }
    let (os, arch) = platform()?;
    match provider {
        | Provider::Claude => {
            let base = "https://downloads.claude.ai/claude-code-releases";
            let version = if let Some(version) = requested {
                version.into()
            } else {
                String::from_utf8(get(client, &format!("{base}/latest"), 128)?)?.trim().to_owned()
            };
            ensure!(version_ok(&version), "Invalid Claude release version");
            let target = format!(
                "{}-{}",
                if os == "windows" { "win32" } else { os },
                if arch == "x86_64" { "x64" } else { "arm64" }
            );
            let manifest = json(client, &format!("{base}/{version}/manifest.json"))?;
            ensure!(manifest["version"] == version, "Claude manifest version mismatch");
            let entry = &manifest["platforms"][&target];
            ensure!(entry["binary"] == binary(provider), "Unsupported Claude target");
            Ok(Release {
                public: RuntimeRelease {
                    url: format!("{base}/{version}/{target}/{}", binary(provider)),
                    version,
                    sha256: entry["checksum"].as_str().context("Missing Claude checksum")?.into(),
                    size: entry["size"].as_u64(),
                },
                format: "binary",
                source_binary: binary(provider).into(),
            })
        },
        | Provider::Codex => {
            let endpoint = requested
                .map(|v| format!("https://api.github.com/repos/openai/codex/releases/tags/rust-v{v}"))
                .unwrap_or_else(|| "https://api.github.com/repos/openai/codex/releases/latest".into());
            let manifest = json(client, &endpoint)?;
            let version = manifest["tag_name"]
                .as_str()
                .and_then(|v| v.strip_prefix("rust-v"))
                .context("Invalid Codex release tag")?
                .to_owned();
            ensure!(version_ok(&version) && requested.is_none_or(|v| v == version), "Codex release version mismatch");
            let target = match os {
                | "windows" => format!("{arch}-pc-windows-msvc"),
                | "darwin" => format!("{arch}-apple-darwin"),
                | _ => format!("{arch}-unknown-linux-musl"),
            };
            let source_binary = format!("codex-{target}{}", if os == "windows" { ".exe" } else { "" });
            let archive = format!("{source_binary}{}", if os == "windows" { ".zip" } else { ".tar.gz" });
            let asset = manifest["assets"]
                .as_array()
                .context("Missing Codex assets")?
                .iter()
                .find(|a| a["name"] == archive)
                .context("No supported Codex asset")?;
            let sha = asset["digest"]
                .as_str()
                .and_then(|s| s.strip_prefix("sha256:"))
                .context("Codex release has no SHA-256 digest")?;
            Ok(Release {
                public: RuntimeRelease {
                    version,
                    url: asset["browser_download_url"].as_str().context("Missing Codex asset URL")?.into(),
                    sha256: sha.into(),
                    size: asset["size"].as_u64(),
                },
                format: if os == "windows" { "zip" } else { "tar.gz" },
                source_binary,
            })
        },
        | Provider::Antigravity => {
            let latest = if requested.is_none() { Some(antigravity::latest(client)?) } else { None };
            let releases = antigravity::catalog(settings, latest)?;
            let public = releases
                .into_iter()
                .find(|release| requested.is_none_or(|version| version == release.version))
                .context("Unknown Antigravity version; check available versions first")?;
            Ok(Release {
                public,
                format: "zip",
                source_binary: binary(provider).into(),
            })
        },
    }
}
fn safe_relative(path: &Path) -> Result<PathBuf> {
    let mut clean = PathBuf::new();
    for part in path.components() {
        match part {
            | std::path::Component::Normal(name) => {
                let name = name.to_str().context("Archive path must be UTF-8")?;
                ensure!(!name.contains(['\\', ':']), "Unsafe archive path");
                clean.push(name);
            },
            | std::path::Component::CurDir => {},
            | _ => bail!("Archive path escapes its destination"),
        }
    }
    ensure!(!clean.as_os_str().is_empty(), "Empty archive path");
    Ok(clean)
}
fn extract(archive: &Path, dest: &Path, format: &str) -> Result<()> {
    let mut total = 0u64;
    let mut names = std::collections::HashSet::new();
    let mut copy = |name: PathBuf, size: u64, directory: bool, input: &mut dyn Read| -> Result<()> {
        let name = safe_relative(&name)?;
        ensure!(names.insert(name.clone()) && names.len() <= 2048, "Duplicate or excessive archive entries");
        total = total.checked_add(size).context("Archive size overflow")?;
        ensure!(total <= 1024 * 1024 * 1024, "Extracted runtime exceeds limit");
        let path = dest.join(name);
        if directory {
            std::fs::create_dir_all(path)?;
        } else {
            let parent = match path.parent() {
                | Some(parent) => parent,
                | None => bail!("Archive entry has no parent directory: {}", path.display()),
            };
            std::fs::create_dir_all(parent)?;
            let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            ensure!(std::io::copy(&mut input.take(size + 1), &mut out)? == size, "Invalid archive entry size");
            out.sync_all()?;
        }
        Ok(())
    };
    if format == "zip" {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
        for i in 0 .. zip.len() {
            let mut file = zip.by_index(i)?;
            ensure!(
                file.unix_mode().is_none_or(|mode| mode & 0o170000 != 0o120000),
                "Archive symlinks are unsupported"
            );
            let name = file.enclosed_name().context("Unsafe ZIP path")?.to_path_buf();
            copy(name, file.size(), file.is_dir(), &mut file)?;
        }
    } else {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(std::fs::File::open(archive)?));
        for entry in tar.entries()? {
            let mut entry = entry?;
            let kind = entry.header().entry_type();
            ensure!(kind.is_file() || kind.is_dir(), "Archive links and devices are unsupported");
            copy(entry.path()?.into_owned(), entry.size(), kind.is_dir(), &mut entry)?;
        }
    }
    Ok(())
}
fn download_verified(mut response: impl Read, archive: &Path, release: &RuntimeRelease) -> Result<()> {
    let mut file = std::fs::File::create(archive)?;
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut chunk = [0; 65536];
    loop {
        let n = response.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        ensure!(count <= DOWNLOAD_LIMIT, "Download exceeds limit");
        hash.update(&chunk[.. n]);
        file.write_all(&chunk[.. n])?;
    }
    file.sync_all()?;
    drop(file);
    ensure!(release.size.is_none_or(|size| size == count), "Runtime download size mismatch");
    ensure!(format!("{:x}", hash.finalize()) == release.sha256, "Runtime SHA-256 mismatch");
    Ok(())
}
fn install(settings: &Settings, provider: Provider, release: &Release, client: &reqwest::blocking::Client) -> Result<()> {
    let root = root(settings, provider);
    std::fs::create_dir_all(&root)?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("install.lock"))?;
    lock.try_lock().context("Another runtime installation is in progress")?;
    let stage = tempfile::Builder::new().prefix(".staging-").tempdir_in(&root)?;
    let archive = stage.path().join("download");
    let mut response = client.get(&release.public.url).send()?.error_for_status()?;
    ensure!(release.public.size.is_none_or(|n| n <= DOWNLOAD_LIMIT), "Download exceeds limit");
    download_verified(&mut response, &archive, &release.public)?;
    let files = stage.path().join("files");
    std::fs::create_dir(&files)?;
    if release.format == "binary" {
        std::fs::rename(&archive, files.join(binary(provider)))?;
    } else {
        extract(&archive, &files, release.format)?;
        let source = files.join(&release.source_binary);
        let target = files.join(binary(provider));
        if source != target {
            std::fs::rename(source, target)?;
        }
    }
    let executable = files.join(binary(provider));
    ensure!(executable.is_file(), "Runtime executable missing");
    let harness = (provider == Provider::Antigravity).then(|| {
        files.join(if cfg!(windows) {
            "localharness_external.exe"
        } else {
            "localharness_external"
        })
    });
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in std::iter::once(&executable).chain(harness.iter()) {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    let validation = tempfile::tempdir()?;
    let agent = gpui_agent::Agent::new(gpui_agent::Config {
        provider,
        executable,
        home: validation.path().join("profile"),
        cwd: validation.path().into(),
        harness,
        browser_helper: Some(settings.browser_helper.clone()),
        timeout: Duration::from_secs(60),
        mcp_home: None,
        mcp_selection: None,
        plugin_mcp: vec![],
    })?;
    tokio::runtime::Runtime::new()?.block_on(agent.validate_runtime())?;
    let directory = format!("{}-{}", release.public.version, &crate::new_token()?[.. 12]);
    let destination = root.join(&directory);
    std::fs::rename(&files, &destination)?;
    let mut pointer = tempfile::NamedTempFile::new_in(&root)?;
    serde_json::to_writer(
        &mut pointer,
        &serde_json::json!({"version":release.public.version,"directory":directory,"sha256":release.public.sha256}),
    )?;
    pointer.as_file().sync_all()?;
    pointer.persist(root.join("current.json")).map_err(|e| e.error)?;
    Ok(())
}
pub(crate) fn run(settings: &Settings, provider: Provider, command: &RuntimeCommand) -> Result<RuntimeInfo> {
    let override_active = std::env::var_os(format!("gpui_agent_{}_EXECUTABLE", provider_name(provider).to_ascii_uppercase())).is_some()
        || (provider == Provider::Antigravity && std::env::var_os("gpui_agent_ANTIGRAVITY_HARNESS").is_some());
    let mut release_info = None;
    if !matches!(command, RuntimeCommand::Inspect) {
        let client = client()?;
        let version = match command {
            | RuntimeCommand::Install {
                version, ..
            } => Some(version.as_str()),
            | _ => None,
        };
        let release = release(&client, settings, provider, version)?;
        ensure!(
            release.public.sha256.len() == 64 && release.public.sha256.bytes().all(|b| b.is_ascii_hexdigit()) && allowed(&release.public.url.parse()?),
            "Invalid release metadata"
        );
        if let RuntimeCommand::Install {
            sha256, ..
        } = command
        {
            ensure!(
                sha256 == &release.public.sha256,
                "Release checksum changed since review; check for updates again"
            );
            ensure!(
                !override_active,
                "An executable override is active; remove it before installing managed runtimes"
            );
            install(settings, provider, &release, &client)?;
        }
        release_info = Some(release.public);
    }
    let releases = if provider == Provider::Antigravity {
        antigravity::catalog(settings, None)?
    } else {
        vec![]
    };
    let runtime_issue = settings.agent(provider).err().map(|e| e.to_string());
    let executable = settings.executable(provider);
    Ok(RuntimeInfo {
        provider,
        active_path: executable.as_ref().ok().map(|p| p.display().to_string()),
        managed_version: selected(settings, provider)?.map(|(version, _)| version),
        override_active,
        release: release_info.or_else(|| releases.first().cloned()),
        releases,
        issue: runtime_issue.or_else(|| executable.err().map(|e| e.to_string())),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cached_applications_recheck_runtime_availability_before_saving_a_turn() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let settings = Settings::discover(temp.path().into(), Some(temp.path().join("home")))?;
        let base = root(&settings, Provider::Claude);
        std::fs::create_dir_all(&base)?;
        std::fs::write(base.join("current.json"), br#"{"version":"../bad"}"#)?;
        let app = settings.application()?;
        assert!(!app.providers()[0].available);
        let request = gpui_agent_contracts::ChatRequest {
            extensions: None,
            skills: vec![],
            computer_use: false,
            claude_permission_mode: None,
            codex_execution_policy: None,
            text: "must not save".into(),
            model: None,
            session_id: None,
            conversation_id: None,
            attachments: vec![],
        };
        assert!(matches!(
            app.chat(Provider::Claude, request).await,
            Err(gpui_agent_application::ApplicationError(gpui_agent_application::ErrorKind::Unavailable, _))
        ));
        assert!(app.conversations(Provider::Claude, String::new()).await?.is_empty());
        std::fs::create_dir(base.join("1.0.0"))?;
        std::fs::write(base.join("1.0.0").join(binary(Provider::Claude)), b"metadata fixture only")?;
        std::fs::write(base.join("current.json"), br#"{"version":"1.0.0"}"#)?;
        assert!(app.providers()[0].available);
        Ok(())
    }

    #[test]
    fn verifies_download_bytes_and_rejects_hostile_archives() -> Result<()> {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("archive");
        let bytes = b"verified runtime";
        let release = RuntimeRelease {
            version: "1.0.0".into(),
            url: "https://dl.google.com/test".into(),
            size: Some(bytes.len() as u64),
            sha256: format!("{:x}", Sha256::digest(bytes)),
        };
        download_verified(bytes.as_slice(), &archive, &release)?;
        assert!(download_verified(b"damaged".as_slice(), &archive, &release).is_err());
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive)?);
        zip.start_file("../escape", zip::write::SimpleFileOptions::default())?;
        zip.write_all(b"bad")?;
        zip.finish()?;
        let destination = root.path().join("out");
        std::fs::create_dir(&destination)?;
        assert!(extract(&archive, &destination, "zip").is_err());
        assert!(!root.path().join("escape").exists());
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive)?);
        zip.start_file("bin/runtime", zip::write::SimpleFileOptions::default())?;
        zip.write_all(b"safe")?;
        zip.finish()?;
        extract(&archive, &destination, "zip")?;
        assert_eq!(std::fs::read(destination.join("bin/runtime"))?, b"safe");
        Ok(())
    }
    #[test]
    fn managed_pointer_rejects_path_escape_and_survives_new_settings() -> Result<()> {
        let temp_root = tempfile::tempdir()?;
        let settings = Settings::discover(temp_root.path().into(), Some(temp_root.path().join("home")))?;
        let provider = Provider::Claude;
        let base = root(&settings, provider);
        std::fs::create_dir_all(base.join("2.0.0-first"))?;
        std::fs::write(base.join("2.0.0-first").join(binary(provider)), b"old")?;
        std::fs::write(base.join("current.json"), br#"{"version":"2.0.0","directory":"2.0.0-first"}"#)?;
        assert_eq!(selected(&settings, provider)?.unwrap().0, "2.0.0");
        assert!(managed_path(&settings, provider)?.unwrap().is_file());
        std::fs::write(base.join("current.json"), br#"{"version":"2.0.0","directory":"../escape"}"#)?;
        assert!(managed_path(&settings, provider).is_err());
        Ok(())
    }

    #[test]
    fn versions_and_archive_paths_cannot_escape() {
        for v in ["../bad", "--help", "1/2", "1\\2", ""] {
            assert!(!version_ok(v));
        }
        for p in ["../bad", "/bad", "C:/bad", "dir/../../bad", "dir\\..\\..\\bad"] {
            assert!(safe_relative(Path::new(p)).is_err());
        }
        assert!(safe_relative(Path::new("bin/codex")).is_ok());
        assert!(!allowed(&"https://evil.test/runtime".parse().unwrap()));
    }
}
