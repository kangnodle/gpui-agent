//! Installed package copies; no package code is executed during management.
use anyhow::{Context,
             Result,
             ensure};
use gpui_agent_application::ports::PluginSource;
use gpui_agent_domain::*;
use sha2::{Digest,
           Sha256};
use std::{collections::BTreeMap,
          fs,
          io::{Read,
               Write},
          path::{Path,
                 PathBuf}};

pub struct PluginFiles {
    home: PathBuf,
    root: PathBuf,
    repo_approval_directory: Option<PathBuf>,
}
impl PluginFiles {
    pub fn new(home: &Path, cwd: &Path) -> Result<Self> {
        fs::create_dir_all(home)?;
        let root = cwd.canonicalize()?;
        let configured_home = std::path::absolute(home)?;
        let home = configured_home.canonicalize()?;
        let workspace = Sha256::digest(root.as_os_str().as_encoded_bytes());
        let directory = home.join("repo-plugin-approvals").join(format!("{workspace:x}"));
        let mut repo_approval_directory = (!directory.starts_with(&root)).then_some(directory);
        for ancestor in configured_home.ancestors() {
            if ancestor.canonicalize()?.starts_with(&root) {
                repo_approval_directory = None;
                break;
            }
        }
        Ok(Self {
            home,
            root,
            repo_approval_directory,
        })
    }

    fn regular(path: &Path) -> Result<fs::Metadata> {
        let metadata = fs::symlink_metadata(path)?;
        ensure!(!metadata.file_type().is_symlink(), "Linked plugin paths are not supported");
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(metadata.file_attributes() & 0x400 == 0, "Reparse plugin paths are not supported");
        }
        ensure!(metadata.is_file() || metadata.is_dir(), "Plugin paths must be regular files or directories");
        Ok(metadata)
    }

    fn checked(path: &Path, base: &Path, create: bool) -> Result<()> {
        let mut current = base.to_path_buf();
        Self::regular(&current)?;
        for part in path.strip_prefix(base)?.components() {
            current.push(part);
            match Self::regular(&current) {
                | Ok(_) => ensure!(current.canonicalize()?.starts_with(base), "Plugin path escapes its scope"),
                | Err(error) if error.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
                    if create {
                        fs::create_dir(&current)?;
                    },
                | Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn base(&self, scope: &str) -> Result<&Path> {
        match scope {
            | "user" => Ok(&self.home),
            | "repo" => Ok(&self.root),
            | _ => anyhow::bail!("Unknown plugin scope"),
        }
    }

    fn directory(&self, scope: &str) -> Result<PathBuf> { Ok(self.base(scope)?.join(if scope == "user" { "plugins" } else { ".agents/plugins" })) }

    fn settings_path(&self, scope: &str) -> Result<PathBuf> {
        Ok(self.base(scope)?.join(if scope == "user" {
            "plugin-settings.json"
        } else {
            ".agents/plugin-settings.json"
        }))
    }

    fn settings(&self, scope: &str) -> Result<BTreeMap<String, bool>> {
        let path = self.settings_path(scope)?;
        Self::checked(&path, self.base(scope)?, false)?;
        match fs::File::open(path) {
            | Ok(file) => {
                ensure!(file.metadata()?.len() <= 256 * 1024, "Plugin settings exceed capacity");
                Ok(serde_json::from_reader(file)?)
            },
            | Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            | Err(error) => Err(error.into()),
        }
    }

    fn save_settings(&self, scope: &str, settings: &BTreeMap<String, bool>) -> Result<()> {
        let path = self.settings_path(scope)?;
        let bytes = serde_json::to_vec(settings)?;
        ensure!(bytes.len() <= 256 * 1024, "Plugin settings exceed capacity");
        Self::write_settings(&path, self.base(scope)?, &bytes)
    }

    fn write_settings(path: &Path, base: &Path, bytes: &[u8]) -> Result<()> {
        let parent = path.parent().context("Missing plugin settings parent")?;
        Self::checked(parent, base, true)?;
        Self::checked(path, base, false)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(bytes)?;
        file.as_file().sync_all()?;
        file.persist(path)?;
        Ok(())
    }

    fn mcp_fingerprint(servers: &[McpServer]) -> Result<String> { Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(servers)?))) }

    fn repo_approval(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let Some(directory) = &self.repo_approval_directory else {
            return Ok(None);
        };
        let path = directory.join(name);
        Self::checked(&path, &self.home, false)?;
        match fs::File::open(path) {
            | Ok(file) => {
                let mut bytes = Vec::new();
                file.take(65).read_to_end(&mut bytes)?;
                Ok(Some(bytes))
            },
            | Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            | Err(error) => Err(error.into()),
        }
    }

    fn update_repo_approval(&self, scope: &str, name: &str, servers: &[McpServer]) -> Result<()> {
        if scope != "repo" {
            return Ok(());
        }
        let Some(directory) = &self.repo_approval_directory else {
            ensure!(
                servers.is_empty(),
                "Select a guidance home outside this workspace to approve plugin MCP servers"
            );
            return Ok(());
        };
        let path = directory.join(name);
        if servers.is_empty() {
            Self::checked(&path, &self.home, false)?;
            return match fs::remove_file(path) {
                | Ok(()) => Ok(()),
                | Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                | Err(error) => Err(error.into()),
            };
        }
        Self::write_settings(&path, &self.home, Self::mcp_fingerprint(servers)?.as_bytes())
    }

    pub(crate) fn lock(&self) -> Result<fs::File> {
        let path = self.home.join(".plugins.lock");
        Self::checked(&path, &self.home, false)?;
        let lock = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
        lock.lock()?;
        Ok(lock)
    }

    pub fn collect_plugin(directory: &Path, scope: String) -> Result<PluginImportRequest> {
        let claude = directory.join(".claude-plugin/plugin.json");
        let request = if !directory.join("plugin.json").exists() && claude.is_file() {
            Self::read_claude_bundle(directory, &claude, scope)?
        } else {
            Self::read_bundle(directory, directory, scope)?
        };
        request.validate().map_err(anyhow::Error::msg)?;
        Ok(request)
    }

    /// Converts a Claude Code plugin folder: only its manifest identity and
    /// `skills/` are carried over. Hooks, scripts and other code never are.
    fn read_claude_bundle(directory: &Path, manifest: &Path, scope: String) -> Result<PluginImportRequest> {
        ensure!(Self::regular(manifest)?.len() <= 64 * 1024, "Claude plugin manifest is too large");
        let manifest: serde_json::Value = serde_json::from_slice(&fs::read(manifest)?).context("Invalid Claude plugin manifest")?;
        let text = |key| manifest.get(key).and_then(|value| value.as_str()).unwrap_or_default();
        let native = serde_json::to_vec(&serde_json::json!({
            "name": text("name"),
            "description": text("description"),
            "version": text("version"),
        }))?;
        let skills = directory.join("skills");
        ensure!(skills.exists(), "Claude plugin has no skills to import");
        let mut request = Self::read_bundle(directory, &skills, scope)?;
        request.files.insert("plugin.json".into(), native);
        Ok(request)
    }

    fn read_bundle(directory: &Path, start: &Path, scope: String) -> Result<PluginImportRequest> {
        let mut files = BTreeMap::new();
        let mut pending = vec![(start.to_path_buf(), 0usize)];
        let mut entries = 0;
        let mut total = 0;
        while let Some((path, depth)) = pending.pop() {
            entries += 1;
            ensure!(entries <= 512 && depth <= 6, "Plugin folder exceeds entry or depth limit");
            let metadata = Self::regular(&path)?;
            if metadata.is_dir() {
                for entry in fs::read_dir(&path)? {
                    pending.push((entry?.path(), depth + 1));
                    ensure!(pending.len() <= 512, "Plugin folder exceeds entry limit");
                }
            } else {
                ensure!(
                    metadata.len() <= MAX_SKILL_BYTES as u64 && files.len() < 128,
                    "Plugin resource exceeds capacity"
                );
                let mut bytes = Vec::new();
                fs::File::open(&path)?.take((MAX_SKILL_BYTES + 1) as u64).read_to_end(&mut bytes)?;
                total += bytes.len();
                ensure!(total <= 1024 * 1024, "Plugin folder exceeds 1 MiB");
                files.insert(
                    path.strip_prefix(directory)?.to_str().context("Plugin paths require UTF-8")?.replace('\\', "/"),
                    bytes,
                );
            }
        }
        Ok(PluginImportRequest {
            scope,
            files,
        })
    }

    fn loaded(&self, directory: &Path, scope: &str, enabled: bool) -> Result<Plugin> {
        let request = Self::collect_plugin(directory, scope.into())?;
        let mut plugin = request.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            directory.file_name().and_then(|name| name.to_str()) == Some(plugin.manifest.name.as_str()),
            "Installed plugin name differs from directory"
        );
        plugin.enabled = if scope == "repo" && !plugin.manifest.mcp_servers.is_empty() {
            self.repo_approval(&plugin.manifest.name)?.as_deref() == Some(Self::mcp_fingerprint(&plugin.manifest.mcp_servers)?.as_bytes())
        } else {
            enabled
        };
        for skill in &mut plugin.skills {
            let path = skill.id.strip_prefix(&format!("plugin:{}/", plugin.id)).context("Invalid plugin skill ID")?;
            let auxiliary = request
                .files
                .get(&format!("{}/agents/openai.yaml", path.strip_suffix("/SKILL.md").context("Invalid skill path")?));
            let text = std::str::from_utf8(request.files.get(path).context("Missing plugin skill")?)?;
            let auxiliary = auxiliary.map(|b| std::str::from_utf8(b)).transpose()?.unwrap_or_default();
            skill.revision = format!("{:x}", Sha256::digest(format!("{text}\n{auxiliary}")));
        }
        Ok(plugin)
    }

    /// Claude provider profiles: the default one and one per account.
    fn claude_profiles(&self) -> Result<Vec<PathBuf>> {
        let mut homes = vec![self.home.clone()];
        match fs::read_dir(self.home.join("accounts")) {
            | Ok(entries) => {
                let entries = entries.take(257).collect::<std::io::Result<Vec<_>>>()?;
                ensure!(entries.len() <= 256, "Too many account profiles");
                for entry in entries {
                    if entry.file_type()?.is_dir() {
                        homes.push(entry.path());
                    }
                }
            },
            | Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            | Err(error) => return Err(error.into()),
        }
        homes.sort();
        Ok(homes
            .into_iter()
            .map(|home| home.join("claude"))
            .filter(|profile| profile.join("plugins").is_dir())
            .collect())
    }

    /// `(marketplace key, install path)` for each plugin the Claude CLI has
    /// installed, preferring the most recent install of each.
    fn claude_installs(profile: &Path) -> Result<Option<Vec<(String, String)>>> {
        let path = profile.join("plugins/installed_plugins.json");
        let file = match fs::File::open(&path) {
            | Ok(file) => file,
            | Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            | Err(error) => return Err(error.into()),
        };
        ensure!(file.metadata()?.len() <= 1024 * 1024, "Claude plugin list is too large");
        let value: serde_json::Value = serde_json::from_reader(file).context("Invalid Claude plugin list")?;
        let plugins = value
            .get("plugins")
            .and_then(|plugins| plugins.as_object())
            .context("Invalid Claude plugin list")?;
        ensure!(plugins.len() <= 128, "Too many Claude plugins");
        Ok(Some(
            plugins
                .iter()
                .filter_map(|(key, installs)| {
                    let path = installs.as_array()?.iter().rev().find_map(|install| install.get("installPath")?.as_str())?;
                    Some((key.clone(), path.to_owned()))
                })
                .collect(),
        ))
    }

    pub fn mcp_servers(&self) -> Result<Vec<McpServer>> {
        let catalog = self.catalog()?;
        for diagnostic in &catalog.diagnostics {
            let Ok((scope, name)) = validate_plugin_id(&diagnostic.source) else {
                continue;
            };
            let enabled = if scope == "repo" {
                self.repo_approval(name)?.is_some()
            } else {
                self.settings(scope)?.get(name).copied().unwrap_or(false)
            };
            ensure!(!enabled, "Enabled plugin needs attention before using MCP: {}", diagnostic.source);
        }
        let servers: Vec<_> = catalog.plugins.into_iter().filter(|p| p.enabled).flat_map(|p| p.manifest.mcp_servers).collect();
        validate_mcp(&servers).map_err(anyhow::Error::msg)?;
        Ok(servers)
    }

    pub(crate) fn skill_directory(&self, id: &str) -> Result<PathBuf> {
        let rest = id.strip_prefix("plugin:").context("Invalid plugin skill ID")?;
        let (plugin_id, relative) = rest.split_once('/').context("Invalid plugin skill ID")?;
        let (scope, name) = validate_plugin_id(plugin_id).map_err(anyhow::Error::msg)?;
        validate_skill_resource(relative).map_err(anyhow::Error::msg)?;
        ensure!(
            self.catalog()?
                .plugins
                .iter()
                .any(|plugin| plugin.id == plugin_id && plugin.enabled && plugin.skills.iter().any(|s| s.id == id)),
            "Plugin skill is disabled or unavailable"
        );
        let package = self.directory(scope)?.join(name);
        let directory = package.join(relative).parent().context("Missing skill directory")?.to_path_buf();
        Self::checked(&directory, self.base(scope)?, false)?;
        Ok(directory)
    }

    pub(crate) fn read_resource(&self, id: &str, resource: &str) -> Result<PathBuf> {
        validate_skill_resource(resource).map_err(anyhow::Error::msg)?;
        let directory = self.skill_directory(id)?;
        let path = directory.join(resource);
        Self::checked(&path, &directory, false)?;
        Ok(path)
    }
}
impl PluginSource for PluginFiles {
    fn catalog(&self) -> Result<PluginCatalog> {
        let mut catalog = PluginCatalog::default();
        for scope in ["user", "repo"] {
            let directory = self.directory(scope)?;
            Self::checked(&directory, self.base(scope)?, false)?;
            if !directory.exists() {
                continue;
            }
            let settings = self.settings(scope)?;
            let mut entries = fs::read_dir(&directory)?.take(129).collect::<std::io::Result<Vec<_>>>()?;
            ensure!(entries.len() <= 128, "Too many installed plugins");
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                if entry.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let id = format!("{scope}:{name}");
                let result = (|| {
                    validate_plugin_id(&id).map_err(anyhow::Error::msg)?;
                    self.loaded(&entry.path(), scope, settings.get(&name).copied().unwrap_or(false))
                })();
                match result {
                    | Ok(plugin) => catalog.plugins.push(plugin),
                    | Err(error) => catalog.diagnostics.push(GuidanceDiagnostic {
                        source: id,
                        message: error.to_string(),
                    }),
                }
            }
        }
        Ok(catalog)
    }

    fn sync_claude(&self, scope: &str, enable: bool) -> Result<PluginSyncResult> {
        ensure!(matches!(scope, "user" | "repo"), "Select user or repo scope");
        let mut result = PluginSyncResult::default();
        let mut notes = vec![];
        let mut installed: BTreeMap<String, ()> = self.catalog()?.plugins.into_iter().map(|plugin| (plugin.id, ())).collect();
        for profile in self.claude_profiles()? {
            let Some(entries) = Self::claude_installs(&profile)? else {
                continue;
            };
            for (key, install_path) in entries {
                let outcome = (|| {
                    let plugins = profile.join("plugins").canonicalize()?;
                    let directory = Path::new(&install_path).canonicalize()?;
                    ensure!(directory.starts_with(&plugins), "Install path is outside the Claude plugin directory");
                    let request = Self::collect_plugin(&directory, scope.into())?;
                    let plugin = request.validate().map_err(anyhow::Error::msg)?;
                    ensure!(!installed.contains_key(&plugin.id), "Already installed");
                    self.import(&request)?;
                    if enable
                        && plugin.manifest.mcp_servers.is_empty()
                        && let Err(error) = self.set_enabled_reviewed(&plugin.id, true, Some(&[]))
                    {
                        notes.push(GuidanceDiagnostic {
                            source: key.clone(),
                            message: format!("Imported but not enabled: {error}"),
                        });
                    }
                    Ok(plugin.id)
                })();
                match outcome {
                    | Ok(id) => {
                        installed.insert(id.clone(), ());
                        result.imported.push(id);
                    },
                    | Err(error) => result.skipped.push(GuidanceDiagnostic {
                        source: key,
                        message: error.to_string(),
                    }),
                }
            }
        }
        result.skipped.extend(notes);
        result.catalog = self.catalog()?;
        Ok(result)
    }

    fn import(&self, request: &PluginImportRequest) -> Result<PluginCatalog> {
        let plugin = request.validate().map_err(anyhow::Error::msg)?;
        let _lock = self.lock()?;
        let parent = self.directory(&request.scope)?;
        Self::checked(&parent, self.base(&request.scope)?, true)?;
        let target = parent.join(&plugin.manifest.name);
        ensure!(!target.exists(), "Plugin already exists");
        for entry in fs::read_dir(&parent)? {
            ensure!(
                !entry?.file_name().to_string_lossy().eq_ignore_ascii_case(&plugin.manifest.name),
                "Plugin ID collides with an existing directory"
            );
        }
        ensure!(fs::read_dir(&parent)?.count() < 128, "Too many installed plugins");
        let staging = tempfile::tempdir_in(&parent)?;
        for (path, bytes) in &request.files {
            let path = staging.path().join(path);
            fs::create_dir_all(path.parent().context("Missing plugin resource parent")?)?;
            let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        Self::collect_plugin(staging.path(), request.scope.clone())?;
        let mut settings = self.settings(&request.scope)?;
        settings.remove(&plugin.manifest.name);
        self.update_repo_approval(&request.scope, &plugin.manifest.name, &[])?;
        self.save_settings(&request.scope, &settings)?;
        Self::checked(&parent, self.base(&request.scope)?, false)?;
        fs::rename(staging.path(), target)?;
        self.catalog()
    }

    fn set_enabled_reviewed(&self, id: &str, enabled: bool, expected: Option<&[McpServer]>) -> Result<PluginCatalog> {
        let (scope, name) = validate_plugin_id(id).map_err(anyhow::Error::msg)?;
        let _lock = self.lock()?;
        if !enabled {
            let target = self.directory(scope)?.join(name);
            Self::checked(&target, self.base(scope)?, false)?;
            ensure!(Self::regular(&target)?.is_dir(), "Plugin not found");
            self.update_repo_approval(scope, name, &[])?;
            let mut settings = self.settings(scope)?;
            settings.insert(name.into(), false);
            self.save_settings(scope, &settings)?;
            return self.catalog();
        }
        let mut catalog = self.catalog()?;
        let plugin = catalog.plugins.iter_mut().find(|p| p.id == id).context("Plugin not found")?;
        let approved = plugin.manifest.mcp_servers.clone();
        if let Some(expected) = expected {
            ensure!(
                Self::mcp_fingerprint(expected)? == Self::mcp_fingerprint(&approved)?,
                "Plugin MCP configuration changed. Review the current configuration before enabling it."
            );
        }
        plugin.enabled = enabled;
        let servers: Vec<_> = catalog
            .plugins
            .iter()
            .filter(|p| p.enabled)
            .flat_map(|p| p.manifest.mcp_servers.clone())
            .collect();
        validate_mcp(&servers).map_err(anyhow::Error::msg)?;
        let mut effective = gpui_agent::load_mcp_config(&self.home)?;
        effective.extend(servers);
        validate_mcp(&effective).map_err(|error| anyhow::anyhow!("Plugin MCP conflicts with common settings: {error}"))?;
        self.update_repo_approval(scope, name, &approved)?;
        if scope != "repo" || approved.is_empty() {
            let mut settings = self.settings(scope)?;
            settings.insert(name.into(), enabled);
            self.save_settings(scope, &settings)?;
        }
        self.catalog()
    }

    fn remove(&self, id: &str) -> Result<PluginCatalog> {
        let (scope, name) = validate_plugin_id(id).map_err(anyhow::Error::msg)?;
        let _lock = self.lock()?;
        let parent = self.directory(scope)?;
        let target = parent.join(name);
        Self::checked(&target, self.base(scope)?, false)?;
        Self::read_bundle(&target, &target, scope.into())?;
        let mut settings = self.settings(scope)?;
        settings.remove(name);
        self.update_repo_approval(scope, name, &[])?;
        self.save_settings(scope, &settings)?;
        let removed = tempfile::tempdir_in(&parent)?;
        // Publish removal atomically; TempDir owns deletion inside this scope.
        fs::rename(target, removed.path().join("removed"))?;
        removed.close()?;
        self.catalog()
    }
}
