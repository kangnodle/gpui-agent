//! Bounded host guidance discovery; independent of installed model providers.
use anyhow::{Context,
             Result,
             bail,
             ensure};
use gpui_agent_application::ports::{GuidanceSource,
                                    PluginSource};
use gpui_agent_domain::{GuidanceDiagnostic,
                        GuidanceFile,
                        GuidanceFileCatalog,
                        GuidanceSnapshot,
                        GuidanceWriteRequest,
                        MAX_GUIDANCE_DOCUMENT_BYTES,
                        MAX_SKILL_BYTES,
                        SkillMetadata,
                        SkillResource,
                        validate_guidance_file_id,
                        validate_skill_resource};
use sha2::{Digest,
           Sha256};
use std::{collections::{BTreeMap,
                        HashSet},
          fs,
          io::{Read,
               Write},
          path::{Path,
                 PathBuf}};

pub struct GuidanceFiles {
    home: PathBuf,
    root: PathBuf,
    directories: Vec<PathBuf>,
}

impl GuidanceFiles {
    /// Read a user-selected folder without following links or executing
    /// scripts.
    pub fn collect_skill(directory: &Path, id: String) -> Result<gpui_agent_domain::SkillImportRequest> {
        let mut request = gpui_agent_domain::SkillImportRequest {
            id,
            files: BTreeMap::new(),
        };
        let mut pending = vec![(directory.to_path_buf(), 0usize)];
        let mut total = 0usize;
        let mut entries = 0usize;
        while let Some((path, depth)) = pending.pop() {
            entries += 1;
            ensure!(entries <= 512, "Too many skill entries");
            let metadata = fs::symlink_metadata(&path)?;
            ensure!(!metadata.file_type().is_symlink(), "Linked skill resources cannot be imported");
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                ensure!(metadata.file_attributes() & 0x400 == 0, "Reparse skill resources cannot be imported");
            }
            ensure!(depth <= 6, "Skill folder exceeds depth limit");
            if metadata.is_dir() {
                for entry in fs::read_dir(&path)? {
                    pending.push((entry?.path(), depth + 1));
                    ensure!(pending.len() <= 512, "Too many skill entries");
                }
            } else {
                ensure!(
                    metadata.is_file() && metadata.len() <= MAX_SKILL_BYTES as u64,
                    "Skill resource exceeds size limit"
                );
                let mut bytes = Vec::new();
                fs::File::open(&path)?.take((MAX_SKILL_BYTES + 1) as u64).read_to_end(&mut bytes)?;
                total += bytes.len();
                ensure!(total <= 1024 * 1024 && request.files.len() < 128, "Skill folder exceeds capacity");
                request.files.insert(
                    path.strip_prefix(directory)?.to_str().context("Skill path must be UTF-8")?.replace('\\', "/"),
                    bytes,
                );
            }
        }
        request.validate().map_err(anyhow::Error::msg)?;
        Ok(request)
    }

    fn editable_path(&self, id: &str) -> Result<(PathBuf, &Path)> {
        let (scope, relative) = validate_guidance_file_id(id).map_err(anyhow::Error::msg)?;
        let base = if scope == "user" { &self.home } else { &self.root };
        let path = base.join(relative);
        let permitted = if relative.ends_with("/SKILL.md") {
            let roots = if scope == "user" {
                vec![self.home.join("skills")]
            } else {
                self.directories.iter().map(|directory| directory.join(".agents/skills")).collect()
            };
            roots.iter().any(|root| {
                path.strip_prefix(root).is_ok_and(|rest| {
                    let parts: Vec<_> = rest.components().collect();
                    (2 ..= 7).contains(&parts.len()) && parts.iter().all(|part| !part.as_os_str().to_string_lossy().starts_with('.'))
                })
            })
        } else if scope == "user" {
            path.parent() == Some(self.home.as_path())
        } else {
            self.directories.iter().any(|directory| path.parent() == Some(directory.as_path()))
        };
        ensure!(permitted, "Guidance file is outside this workspace's discovery scope");
        Ok((path, base))
    }

    fn check_editor_path(path: &Path, base: &Path, create: bool) -> Result<()> {
        let mut current = base.to_path_buf();
        for part in path.strip_prefix(base)?.components() {
            current.push(part);
            match fs::symlink_metadata(&current) {
                | Ok(metadata) => {
                    ensure!(!metadata.file_type().is_symlink(), "Linked guidance paths cannot be edited");
                    #[cfg(windows)]
                    {
                        use std::os::windows::fs::MetadataExt;
                        ensure!(metadata.file_attributes() & 0x400 == 0, "Reparse guidance paths cannot be edited");
                    }
                    Self::resolve(&current, base)?;
                },
                | Err(error) if error.kind() == std::io::ErrorKind::NotFound =>
                    if create && current != path {
                        fs::create_dir(&current)?;
                    },
                | Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    pub fn new(home: &Path, cwd: &Path) -> Result<Self> {
        fs::create_dir_all(home)?;
        let home = home.canonicalize()?;
        let cwd = cwd.canonicalize()?;
        ensure!(cwd.is_dir(), "Guidance workspace must be a directory");
        let root = cwd.ancestors().find(|path| path.join(".git").exists()).unwrap_or(&cwd).to_path_buf();
        let mut directories = vec![cwd.clone()];
        while directories.last().is_some_and(|path| path != &root) {
            let parent = directories
                .last()
                .and_then(|path| path.parent())
                .context("Invalid guidance root")?
                .to_path_buf();
            directories.push(parent);
        }
        directories.reverse();
        Ok(Self {
            home,
            root,
            directories,
        })
    }

    fn resolve(path: &Path, allowed: &Path) -> Result<PathBuf> {
        let resolved = path.canonicalize().context("Guidance file is unavailable")?;
        ensure!(resolved.starts_with(allowed), "Guidance path escapes its allowed root");
        ensure!(
            !resolved
                .strip_prefix(allowed)?
                .components()
                .any(|part| part.as_os_str().eq_ignore_ascii_case(".git")),
            "Git metadata is not guidance"
        );
        Ok(resolved)
    }

    fn read(path: &Path, allowed: &Path, limit: usize) -> Result<String> {
        let resolved = Self::resolve(path, allowed)?;
        ensure!(fs::metadata(&resolved)?.is_file(), "Guidance source must be a regular file");
        let file = fs::File::open(&resolved)?;
        ensure!(file.metadata()?.is_file(), "Guidance source must be a regular file");
        let mut bytes = Vec::new();
        file.take((limit + 1) as u64).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= limit, "Guidance file exceeds its size limit");
        ensure!(Self::resolve(path, allowed)? == resolved, "Guidance path changed while reading");
        String::from_utf8(bytes).context("Guidance must be UTF-8 text")
    }

    fn optional(path: &Path, allowed: &Path, limit: usize) -> Result<Option<String>> {
        match fs::symlink_metadata(path) {
            | Ok(_) => Ok(Some(Self::read(path, allowed, limit)?)),
            | Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            | Err(error) => Err(error.into()),
        }
    }

    fn diagnostic(snapshot: &mut GuidanceSnapshot, source: String, error: impl std::fmt::Display) {
        snapshot.diagnostics.push(GuidanceDiagnostic {
            source,
            message: error.to_string(),
        });
    }

    fn instructions(&self, snapshot: &mut GuidanceSnapshot, folder: &Path, allowed: &Path, prefix: &str, global: bool) {
        for name in ["AGENTS.override.md", "AGENTS.md"] {
            let path = folder.join(name);
            let source = format!(
                "{prefix}{}",
                path.strip_prefix(allowed).unwrap_or(Path::new(name)).to_string_lossy().replace('\\', "/")
            );
            // Read a bounded prefix, then let pure policy apply the shared
            // budget.
            let result = (|| -> Result<Option<String>> {
                if fs::symlink_metadata(&path).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) {
                    return Ok(None);
                }
                let resolved = Self::resolve(&path, allowed)?;
                if !fs::metadata(&resolved)?.is_file() {
                    return Ok(None);
                }
                let file = fs::File::open(&resolved)?;
                ensure!(file.metadata()?.is_file(), "Instruction source must be a regular file");
                let mut bytes = Vec::new();
                file.take((MAX_GUIDANCE_DOCUMENT_BYTES + 4) as u64).read_to_end(&mut bytes)?;
                while std::str::from_utf8(&bytes).is_err_and(|error| error.error_len().is_none()) {
                    bytes.pop();
                }
                Ok(Some(String::from_utf8(bytes).context("Instructions must be UTF-8 text")?))
            })();
            match result {
                | Ok(None) => continue,
                | Ok(Some(text)) if global && text.trim().is_empty() => continue,
                | Ok(Some(text)) => {
                    snapshot.add_instruction(source, text);
                    break;
                },
                | Err(error) => {
                    Self::diagnostic(snapshot, source, error);
                    break;
                },
            }
        }
    }

    fn settings(&self) -> Result<BTreeMap<String, bool>> {
        Self::optional(&self.home.join("skill-settings.json"), &self.home, 256 * 1024)?
            .map(|text| serde_json::from_str(&text).context("Invalid host skill settings"))
            .transpose()
            .map(Option::unwrap_or_default)
    }

    fn setting_key(&self, id: &str) -> String {
        if id.starts_with("user:") || id.starts_with("plugin:user:") {
            id.to_owned()
        } else if id.starts_with("plugin:repo:") {
            format!("{}:{id}", self.directories.last().unwrap_or(&self.root).display())
        } else {
            format!("{}:{id}", self.root.display())
        }
    }

    fn manifest(&self, id: &str) -> Result<(PathBuf, PathBuf)> {
        let (prefix, relative) = id.split_once(':').context("Unknown skill ID")?;
        validate_skill_resource(relative).map_err(anyhow::Error::msg)?;
        ensure!(relative.ends_with("/SKILL.md"), "Unknown skill ID");
        let base = match prefix {
            | "user" => &self.home,
            | "repo" => &self.root,
            | _ => bail!("Unknown skill scope"),
        };
        let path = base.join(relative);
        let directory = Self::resolve(path.parent().context("Skill directory is unavailable")?, base)?;
        Ok((Self::resolve(&path, &directory)?, directory))
    }
}

impl GuidanceSource for GuidanceFiles {
    fn import_skill(&self, request: &gpui_agent_domain::SkillImportRequest) -> Result<GuidanceFile> {
        request.validate().map_err(anyhow::Error::msg)?;
        let (manifest, base) = self.editable_path(&request.id)?;
        let target = manifest.parent().context("Missing skill directory")?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.home.join(".guidance.lock"))?;
        lock.lock()?;
        ensure!(!target.exists(), "Skill already exists; choose a different folder name");
        Self::check_editor_path(target, base, true)?;
        let parent = target.parent().context("Missing skills directory")?;
        let staging = tempfile::tempdir_in(parent)?;
        for (name, bytes) in &request.files {
            let destination = staging.path().join(name);
            fs::create_dir_all(destination.parent().context("Missing resource directory")?)?;
            let mut file = fs::OpenOptions::new().write(true).create_new(true).open(destination)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        Self::check_editor_path(target, base, false)?;
        ensure!(!target.exists(), "Skill already exists");
        fs::rename(staging.path(), target)?;
        self.read_file(&request.id)
    }

    fn files(&self) -> Result<GuidanceFileCatalog> {
        let snapshot = self.snapshot()?;
        let mut files: Vec<_> = snapshot.skills.into_iter().map(|skill| skill.id).collect();
        files.extend(
            snapshot
                .diagnostics
                .into_iter()
                .map(|diagnostic| diagnostic.source)
                .filter(|id| id.ends_with("/SKILL.md") && self.editable_path(id).is_ok()),
        );
        for (directory, base, prefix) in
            std::iter::once((&self.home, &self.home, "user")).chain(self.directories.iter().map(|directory| (directory, &self.root, "repo")))
        {
            for name in ["AGENTS.md", "AGENTS.override.md"] {
                let path = directory.join(name);
                if fs::symlink_metadata(&path).is_ok() {
                    files.push(format!("{prefix}:{}", path.strip_prefix(base)?.to_string_lossy().replace('\\', "/")));
                }
            }
        }
        files.sort();
        files.dedup();
        let relative = self
            .directories
            .last()
            .context("Missing workspace")?
            .strip_prefix(&self.root)?
            .to_string_lossy()
            .replace('\\', "/");
        Ok(GuidanceFileCatalog {
            files,
            project_prefix: if relative.is_empty() { relative } else { format!("{relative}/") },
        })
    }

    fn read_file(&self, id: &str) -> Result<GuidanceFile> {
        let (path, base) = self.editable_path(id)?;
        Self::check_editor_path(&path, base, false)?;
        let text = Self::optional(&path, base, MAX_SKILL_BYTES)?;
        let revision = text.as_ref().map(|text| format!("{:x}", Sha256::digest(text.as_bytes())));
        Ok(GuidanceFile {
            id: id.into(),
            text: text.unwrap_or_default(),
            revision,
        })
    }

    fn write_file(&self, request: &GuidanceWriteRequest) -> Result<GuidanceFile> {
        request.validate().map_err(anyhow::Error::msg)?;
        let (path, base) = self.editable_path(&request.id)?;
        Self::check_editor_path(&path, base, false)?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.home.join(".guidance.lock"))?;
        lock.lock()?;
        let current = self.read_file(&request.id)?;
        ensure!(
            current.revision == request.expected_revision,
            "Guidance file changed; reload before saving. Your draft has not been saved."
        );
        Self::check_editor_path(&path, base, true)?;
        let mut file = tempfile::NamedTempFile::new_in(path.parent().context("Missing guidance directory")?)?;
        if current.revision.is_some() {
            let permissions = fs::metadata(&path)?.permissions();
            ensure!(!permissions.readonly(), "Guidance file is read-only");
            file.as_file().set_permissions(permissions)?;
        }
        file.write_all(request.text.as_bytes())?;
        file.as_file().sync_all()?;
        Self::check_editor_path(&path, base, false)?;
        ensure!(
            self.read_file(&request.id)?.revision == request.expected_revision,
            "Guidance file changed; reload before saving"
        );
        if current.revision.is_none() {
            file.persist_noclobber(&path)?;
        } else {
            file.persist(&path)?;
        }
        self.read_file(&request.id)
    }

    fn snapshot(&self) -> Result<GuidanceSnapshot> {
        let mut snapshot = GuidanceSnapshot::default();
        self.instructions(&mut snapshot, &self.home, &self.home, "user:", true);
        for directory in &self.directories {
            self.instructions(&mut snapshot, directory, &self.root, "repo:", false);
        }
        let settings = self.settings()?;
        let mut roots = vec![(self.home.join("skills"), &self.home, "user")];
        roots.extend(self.directories.iter().map(|directory| (directory.join(".agents/skills"), &self.root, "repo")));
        let mut visited = HashSet::new();
        let mut manifests = HashSet::new();
        let mut scanned = 0;
        let mut scanned_entries: usize = 0;
        for (start, allowed, prefix) in roots {
            if !start.exists() {
                continue;
            }
            let mut pending = vec![(start, 0)];
            while let Some((directory, depth)) = pending.pop() {
                scanned += 1;
                if scanned > 2000 || snapshot.skills.len() >= 512 || scanned_entries > 20_000 {
                    Self::diagnostic(&mut snapshot, prefix.into(), "Skill discovery limit reached");
                    break;
                }
                let directory = match Self::resolve(&directory, allowed) {
                    | Ok(path) if visited.insert(path.clone()) => path,
                    | Ok(_) => continue,
                    | Err(error) => {
                        Self::diagnostic(&mut snapshot, prefix.into(), error);
                        continue;
                    },
                };
                let manifest = directory.join("SKILL.md");
                if manifest.exists() {
                    let id = format!("{prefix}:{}", manifest.strip_prefix(allowed)?.to_string_lossy().replace('\\', "/"));
                    let loaded = (|| -> Result<SkillMetadata> {
                        let identity = Self::resolve(&manifest, &directory)?;
                        ensure!(manifests.insert(identity), "Duplicate skill manifest");
                        let text = Self::read(&manifest, &directory, MAX_SKILL_BYTES)?;
                        let auxiliary = Self::optional(&directory.join("agents/openai.yaml"), &directory, MAX_GUIDANCE_DOCUMENT_BYTES)?;
                        let name = directory.file_name().and_then(|name| name.to_str()).context("Invalid skill directory name")?;
                        let (mut skill, _) = SkillMetadata::parse(&id, name, &text, auxiliary.as_deref()).map_err(anyhow::Error::msg)?;
                        skill.enabled = settings.get(&self.setting_key(&id)).copied().unwrap_or(true);
                        skill.revision = format!("{:x}", Sha256::digest(format!("{text}\n{}", auxiliary.as_deref().unwrap_or_default())));
                        Ok(skill)
                    })();
                    match loaded {
                        | Ok(skill) => snapshot.skills.push(skill),
                        | Err(error) => Self::diagnostic(&mut snapshot, id, error),
                    }
                }
                if depth >= 6 {
                    continue;
                }
                match fs::read_dir(&directory) {
                    | Ok(entries) => {
                        let mut entries = entries
                            .take(20_000usize.saturating_sub(scanned_entries) + 1)
                            .collect::<std::io::Result<Vec<_>>>()?;
                        scanned_entries += entries.len();
                        if scanned_entries > 20_000 {
                            Self::diagnostic(&mut snapshot, prefix.into(), "Skill directory entry limit reached");
                            break;
                        }
                        entries.sort_by_key(|entry| entry.file_name());
                        for entry in entries.into_iter().rev() {
                            if !entry.file_name().to_string_lossy().starts_with('.') && entry.path().is_dir() {
                                pending.push((entry.path(), depth + 1));
                            }
                        }
                    },
                    | Err(error) => Self::diagnostic(&mut snapshot, prefix.into(), error),
                }
            }
        }
        let plugins = crate::PluginFiles::new(&self.home, self.directories.last().context("Missing workspace")?)?.catalog()?;
        snapshot.diagnostics.extend(plugins.diagnostics);
        for plugin in plugins.plugins.into_iter().filter(|plugin| plugin.enabled) {
            for mut skill in plugin.skills {
                if snapshot.skills.len() >= 512 {
                    Self::diagnostic(&mut snapshot, plugin.id.clone(), "Skill discovery limit reached");
                    break;
                }
                skill.enabled = settings.get(&self.setting_key(&skill.id)).copied().unwrap_or(true);
                snapshot.skills.push(skill);
            }
        }
        snapshot.skills.sort_by(|left, right| left.id.cmp(&right.id));
        snapshot.revision = format!("{:x}", Sha256::digest(serde_json::to_vec(&snapshot)?));
        Ok(snapshot)
    }

    fn read_skill(&self, id: &str, resource: &str) -> Result<SkillResource> {
        validate_skill_resource(resource).map_err(anyhow::Error::msg)?;
        let skill = self
            .snapshot()?
            .skills
            .into_iter()
            .find(|skill| skill.id == id && skill.enabled)
            .context("Skill is unavailable in this workspace")?;
        let plugins = crate::PluginFiles::new(&self.home, self.directories.last().context("Missing workspace")?)?;
        let directory = if id.starts_with("plugin:") {
            plugins.skill_directory(id)?
        } else {
            self.manifest(id)?.1
        };
        if id.starts_with("plugin:") {
            plugins.read_resource(id, resource)?;
        }
        let path = Self::resolve(&directory.join(resource), &directory)?;
        let text = Self::read(&path, &directory, MAX_SKILL_BYTES)?;
        let text = if resource == "SKILL.md" {
            let auxiliary = Self::optional(&directory.join("agents/openai.yaml"), &directory, MAX_GUIDANCE_DOCUMENT_BYTES)?;
            let revision = format!("{:x}", Sha256::digest(format!("{text}\n{}", auxiliary.as_deref().unwrap_or_default())));
            ensure!(revision == skill.revision, "Skill changed while reading; refresh and retry");
            SkillMetadata::parse(id, &skill.name, &text, auxiliary.as_deref())
                .map_err(anyhow::Error::msg)?
                .1
        } else {
            text
        };
        Ok(SkillResource {
            skill,
            resource: resource.into(),
            text,
        })
    }

    fn set_skill_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        ensure!(self.snapshot()?.skills.iter().any(|skill| skill.id == id), "Unknown skill ID");
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.home.join(".guidance.lock"))?;
        lock.lock()?;
        let mut settings = self.settings()?;
        settings.insert(self.setting_key(id), enabled);
        let bytes = serde_json::to_vec(&settings)?;
        ensure!(bytes.len() <= 256 * 1024, "Host skill settings exceed their 256 KiB capacity");
        let mut file = tempfile::NamedTempFile::new_in(&self.home)?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(self.home.join("skill-settings.json"))?;
        Ok(())
    }
}
