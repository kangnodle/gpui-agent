mod workspace;
use anyhow::{Context,
             Result,
             ensure};
use cap_std::fs::Dir;
use gpui_agent_application::ports::FileContextReader;
use gpui_agent_domain::{Attachment,
                        FileContextRequest,
                        WorkspaceFile,
                        WorkspaceFiles,
                        WorkspaceFilesRequest,
                        attachment::{MAX_IMAGE_BYTES,
                                     MAX_TEXT_BYTES,
                                     MAX_WORKSPACE_ENTRIES}};
use std::{fs::File,
          io::{BufRead,
               BufReader,
               Read,
               Seek},
          path::{Path,
                 PathBuf}};
pub(crate) use workspace::PrivateFileIdentity;
use workspace::{Entry,
                Workspace};

pub(crate) struct Files {
    workspace: Workspace,
}
impl Files {
    #[cfg(test)]
    pub(crate) fn new(root: &Path) -> Result<Self> { Self::with_private_paths(&root.canonicalize()?, &[]) }

    pub(crate) fn with_private_paths(root: &Path, private_paths: &[PathBuf]) -> Result<Self> {
        Self::with_private_paths_and_identities(root, private_paths, &[])
    }

    pub(crate) fn with_private_paths_and_identities(root: &Path, private_paths: &[PathBuf], identities: &[PrivateFileIdentity]) -> Result<Self> {
        Ok(Self {
            workspace: Workspace::new(root, private_paths, identities)?,
        })
    }

    fn open_file(&self, request: &FileContextRequest) -> Result<(File, String)> {
        request.validate().map_err(anyhow::Error::msg)?;
        let Entry {
            file,
            path,
            directory,
        } = self.workspace.open(Path::new(&request.path.replace('\\', "/")))?;
        ensure!(!directory, "Context path must be a regular file");
        Ok((file, path.file_name().context("Missing filename")?.to_string_lossy().into_owned()))
    }
}
impl FileContextReader for Files {
    fn attachment(&self, request: &FileContextRequest) -> Result<Attachment> {
        let (mut file, name) = self.open_file(request)?;
        let mut bytes = Vec::new();
        (&mut file).take(12).read_to_end(&mut bytes)?;
        if !matches!(Attachment::from_bytes(name.clone(), &bytes), Ok(Attachment::Image { .. })) {
            file.rewind()?;
            return read_text(file, name, request);
        }
        ensure!(
            request.start_line.is_none() && request.end_line.is_none(),
            "Image attachments do not support line ranges"
        );
        let remaining = MAX_IMAGE_BYTES + 1 - bytes.len();
        file.take(remaining as u64).read_to_end(&mut bytes)?;
        Attachment::from_bytes(name, &bytes).map_err(anyhow::Error::msg)
    }

    fn list(&self, request: &WorkspaceFilesRequest) -> Result<WorkspaceFiles> {
        request.validate().map_err(anyhow::Error::msg)?;
        let path = request
            .path
            .replace('\\', "/")
            .split('/')
            .filter(|part| !part.is_empty() && *part != ".")
            .collect::<Vec<_>>()
            .join("/");
        let entry = self.workspace.open(Path::new(&path))?;
        ensure!(entry.directory, "Context path must be a directory");
        let directory = Dir::from_std_file(entry.file);
        let mut listing = WorkspaceFiles {
            path: path.clone(),
            entries: Vec::new(),
            truncated: false,
        };
        for (index, entry) in directory.entries()?.enumerate() {
            if index == MAX_WORKSPACE_ENTRIES {
                listing.truncated = true;
                break;
            }
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name.contains('\\') {
                continue;
            }
            let relative = if path.is_empty() { name.clone() } else { format!("{path}/{name}") };
            if (WorkspaceFilesRequest {
                path: relative.clone(),
            })
            .validate()
            .is_err()
            {
                continue;
            }
            let Ok(entry) = self.workspace.open_relative(&directory, Path::new(&name), Path::new(&relative)) else {
                continue;
            };
            listing.entries.push(WorkspaceFile {
                path: relative,
                directory: entry.directory,
            });
        }
        listing.entries.sort_by(|a, b| b.directory.cmp(&a.directory).then_with(|| a.path.cmp(&b.path)));
        Ok(listing)
    }

    fn read(&self, request: &FileContextRequest) -> Result<Attachment> {
        let (file, name) = self.open_file(request)?;
        read_text(file, name, request)
    }
}

fn read_text(file: File, name: String, request: &FileContextRequest) -> Result<Attachment> {
    ensure!(file.metadata()?.len() <= 16 * 1024 * 1024, "Context source exceeds 16 MiB");
    let mut text = Vec::new();
    let mut line = Vec::new();
    let start = request.start_line.unwrap_or(1);
    let end = request.end_line.unwrap_or(usize::MAX);
    let mut count = 0;
    let mut reader = BufReader::new(file).take(16 * 1024 * 1024 + 1);
    loop {
        line.clear();
        let size = reader.read_until(b'\n', &mut line)?;
        if size == 0 {
            break;
        }
        count += 1;
        if count >= start {
            ensure!(text.len() + size <= MAX_TEXT_BYTES, "Selected context exceeds 256 KiB; select fewer lines");
            text.extend_from_slice(&line);
        }
        if count == end {
            break;
        }
    }
    ensure!(reader.limit() > 0, "Context source exceeds 16 MiB");
    ensure!(
        count >= start || (count == 0 && start == 1 && request.end_line.is_none()),
        "Start line is beyond the file"
    );
    ensure!(request.end_line.is_none_or(|end| count >= end), "End line is beyond the file");
    let mut attachment = Attachment::from_bytes(name, &text).map_err(anyhow::Error::msg)?;
    let Attachment::Text {
        source, ..
    } = &mut attachment
    else {
        anyhow::bail!("File context requires UTF-8 text; upload images as attachments");
    };
    *source = Some(FileContextRequest {
        path: request.path.replace('\\', "/"),
        start_line: Some(start),
        end_line: Some(count.max(1)),
    });
    Ok(attachment)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    fn link_directory(target: &Path, link: &Path) { std::os::unix::fs::symlink(target, link).unwrap(); }

    #[cfg(windows)]
    fn link_directory(target: &Path, link: &Path) {
        let output = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "New-Item -ItemType Junction -Path $env:T3_CONTEXT_LINK -Target $env:T3_CONTEXT_TARGET -ErrorAction Stop | Out-Null",
            ])
            .env("T3_CONTEXT_LINK", link)
            .env("T3_CONTEXT_TARGET", target)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }

    #[test]
    fn legacy_development_state_is_private() {
        let temp = tempfile::tempdir().unwrap();
        let files = Files::new(temp.path()).unwrap();
        for protected in [".dev/home", ".gpui-agent-dev"] {
            let private = temp.path().join(protected);
            std::fs::create_dir_all(&private).unwrap();
            std::fs::write(private.join("state.txt"), "fixture").unwrap();
            let request = FileContextRequest {
                path: format!("{protected}/state.txt"),
                start_line: None,
                end_line: None,
            };
            assert!(files.read(&request).is_err());
            assert!(files.attachment(&request).is_err());
            assert!(
                files
                    .list(&WorkspaceFilesRequest {
                        path: protected.into()
                    })
                    .is_err()
            );
            let parent = Path::new(protected).parent().unwrap().to_str().unwrap();
            let listing = files
                .list(&WorkspaceFilesRequest {
                    path: parent.into(),
                })
                .unwrap();
            assert!(listing.entries.iter().all(|entry| entry.path != protected));
        }
    }

    #[test]
    fn configured_private_paths_and_hard_link_aliases_are_hidden() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("state");
        let token = temp.path().join("custom.key");
        std::fs::create_dir(&home).unwrap();
        std::fs::write(home.join("history"), "fixture").unwrap();
        let files = Files::with_private_paths(&temp.path().canonicalize().unwrap(), &[home.clone(), token.clone()]).unwrap();
        std::fs::write(&token, "fixture").unwrap();
        std::fs::hard_link(&token, temp.path().join("other.txt")).unwrap();
        std::fs::write(temp.path().join("ordinary.txt"), "first\nsecond\n").unwrap();
        let listing = files.list(&WorkspaceFilesRequest::default()).unwrap();
        assert_eq!(
            listing.entries,
            [WorkspaceFile {
                path: "ordinary.txt".into(),
                directory: false
            }]
        );
        for path in ["state/history", "custom.key", "other.txt"] {
            let request = FileContextRequest {
                path: path.into(),
                start_line: None,
                end_line: None,
            };
            assert!(files.read(&request).is_err());
            assert!(files.attachment(&request).is_err());
        }
        let private_workspace = Files::with_private_paths(&home.canonicalize().unwrap(), std::slice::from_ref(&home)).unwrap();
        assert!(private_workspace.list(&WorkspaceFilesRequest::default()).is_err());
        let range = FileContextRequest {
            path: "ordinary.txt".into(),
            start_line: Some(2),
            end_line: Some(2),
        };
        assert!(matches!(files.attachment(&range).unwrap(), Attachment::Text { text, .. } if text == "second\n"));
    }

    #[cfg(windows)]
    #[test]
    fn private_locations_reject_case_aliases() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("state");
        let token = temp.path().join("custom.key");
        std::fs::create_dir(&home).unwrap();
        std::fs::write(home.join("history"), "fixture").unwrap();
        let files = Files::with_private_paths(temp.path(), &[home, token]).unwrap();
        std::fs::write(temp.path().join("Custom.Key"), "fixture").unwrap();
        for path in ["STATE/HISTORY", "CUSTOM.KEY"] {
            let request = FileContextRequest {
                path: path.into(),
                start_line: None,
                end_line: None,
            };
            assert!(files.read(&request).is_err());
            assert!(files.attachment(&request).is_err());
        }
        assert!(
            files
                .list(&WorkspaceFilesRequest {
                    path: "STATE".into()
                })
                .is_err()
        );
    }

    #[test]
    fn a_renamed_private_file_and_its_original_name_stay_private() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let private = root.join("state");
        std::fs::write(&private, "fixture").unwrap();
        let identity = PrivateFileIdentity::from_file(&File::open(&private).unwrap()).unwrap();
        let files = Files::with_private_paths(&root, std::slice::from_ref(&private)).unwrap();
        std::fs::rename(&private, root.join("previous")).unwrap();
        std::fs::write(&private, "replacement").unwrap();
        let reopened = Files::with_private_paths_and_identities(&root, &[private], &[identity]).unwrap();
        for reader in [files, reopened] {
            assert!(reader.list(&WorkspaceFilesRequest::default()).unwrap().entries.is_empty());
            for path in ["previous", "state"] {
                let request = FileContextRequest {
                    path: path.into(),
                    start_line: None,
                    end_line: None,
                };
                assert!(reader.read(&request).is_err());
                assert!(reader.attachment(&request).is_err());
            }
        }
    }

    #[test]
    fn links_to_an_ordinary_workspace_directory_are_usable() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("folder");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("document"), "sample").unwrap();
        let link = temp.path().join("alias");
        link_directory(&target, &link);
        let files = Files::new(temp.path()).unwrap();
        let request = FileContextRequest {
            path: "alias/document".into(),
            start_line: None,
            end_line: None,
        };
        assert!(matches!(files.attachment(&request).unwrap(), Attachment::Text { text, .. } if text == "sample"));
        let listing = files
            .list(&WorkspaceFilesRequest {
                path: "alias".into(),
            })
            .unwrap();
        assert_eq!(
            listing.entries,
            [WorkspaceFile {
                path: "alias/document".into(),
                directory: false
            }]
        );
        assert!(Files::with_private_paths(&link, &[]).is_err());
        let private = Files::with_private_paths(&temp.path().canonicalize().unwrap(), &[target]).unwrap();
        assert!(private.read(&request).is_err());
        assert!(private.attachment(&request).is_err());
        assert!(
            private
                .list(&WorkspaceFilesRequest {
                    path: "alias".into()
                })
                .is_err()
        );
        assert!(private.list(&WorkspaceFilesRequest::default()).unwrap().entries.is_empty());
    }

    #[test]
    fn replaced_workspace_requires_reselection() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let files = Files::new(&root).unwrap();
        std::fs::rename(&root, temp.path().join("original")).unwrap();
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("example"), "fixture").unwrap();
        assert!(files.list(&WorkspaceFilesRequest::default()).is_err());
        let request = FileContextRequest {
            path: "example".into(),
            start_line: None,
            end_line: None,
        };
        assert!(files.read(&request).is_err());
        assert!(files.attachment(&request).is_err());
    }

    #[test]
    fn captures_ranges_and_enforces_workspace_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("한글.rs"), "one\n둘\nthree\n").unwrap();
        let reader = Files::new(&root).unwrap();
        let request = FileContextRequest {
            path: "한글.rs".into(),
            start_line: Some(2),
            end_line: Some(2),
        };
        let snapshot = reader.read(&request).unwrap();
        assert!(matches!(&snapshot, Attachment::Text { text, .. } if text == "둘\n"));
        std::fs::write(root.join("한글.rs"), "changed").unwrap();
        assert!(matches!(&snapshot, Attachment::Text { text, .. } if text == "둘\n"));
        assert!(reader.read(&request).is_err());
        for path in ["../outside", ".git/config", "C:\\private", "/private", "."] {
            assert!(
                reader
                    .read(&FileContextRequest {
                        path: path.into(),
                        start_line: None,
                        end_line: None
                    })
                    .is_err()
            );
        }
        #[cfg(windows)]
        {
            let outside = temp.path().join("outside");
            std::fs::create_dir(&outside).unwrap();
            std::fs::write(outside.join("private.txt"), "secret").unwrap();
            link_directory(&outside, &root.join("escape"));
            assert!(
                reader
                    .read(&FileContextRequest {
                        path: "escape/private.txt".into(),
                        start_line: None,
                        end_line: None
                    })
                    .is_err()
            );
        }
        #[cfg(unix)]
        {
            std::fs::write(temp.path().join("private"), "secret").unwrap();
            std::os::unix::fs::symlink(temp.path().join("private"), root.join("escape")).unwrap();
            assert!(
                reader
                    .read(&FileContextRequest {
                        path: "escape".into(),
                        start_line: None,
                        end_line: None
                    })
                    .is_err()
            );
        }
        let listing = reader.list(&WorkspaceFilesRequest::default()).unwrap();
        assert!(listing.entries.iter().all(|entry| entry.path != "escape"));
        assert!(
            reader
                .list(&WorkspaceFilesRequest {
                    path: "escape".into()
                })
                .is_err()
        );
    }

    #[test]
    fn workspace_listing_bounds_large_directories_and_sorts_folders_first() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("folder")).unwrap();
        let reader = Files::new(temp.path()).unwrap();
        std::fs::write(temp.path().join("a.txt"), "text").unwrap();
        let listing = reader.list(&WorkspaceFilesRequest::default()).unwrap();
        assert_eq!(
            listing.entries[0],
            WorkspaceFile {
                path: "folder".into(),
                directory: true
            }
        );
        assert!(!listing.truncated);
        for index in 0 .. MAX_WORKSPACE_ENTRIES {
            std::fs::write(temp.path().join(format!("file-{index}.txt")), "").unwrap();
        }
        let listing = reader.list(&WorkspaceFilesRequest::default()).unwrap();
        assert!(listing.truncated);
        assert_eq!(listing.entries.len(), MAX_WORKSPACE_ENTRIES);
    }
}
