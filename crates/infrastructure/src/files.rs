use anyhow::{Context,
             Result,
             ensure};
use gpui_agent_application::ports::FileContextReader;
use gpui_agent_domain::{Attachment,
                        FileContextRequest,
                        WorkspaceFile,
                        WorkspaceFiles,
                        WorkspaceFilesRequest,
                        attachment::{MAX_IMAGE_BYTES,
                                     MAX_TEXT_BYTES,
                                     MAX_WORKSPACE_ENTRIES}};
use std::{io::{BufRead,
               BufReader},
          path::{Path,
                 PathBuf}};

pub(crate) struct Files {
    root: PathBuf,
}
impl Files {
    pub(crate) fn new(root: &Path) -> Result<Self> {
        Ok(Self {
            root: root.canonicalize()?,
        })
    }

    fn resolve(&self, relative: &str) -> Result<PathBuf> {
        let path = self.root.join(relative).canonicalize().context("Context path was not found")?;
        ensure!(path.starts_with(&self.root), "Context file escapes the selected workspace");
        ensure!(
            !path
                .strip_prefix(&self.root)?
                .components()
                .any(|part| part.as_os_str().eq_ignore_ascii_case(".git")),
            "Git metadata cannot be attached"
        );
        Ok(path)
    }
}
impl FileContextReader for Files {
    fn attachment(&self, request: &FileContextRequest) -> Result<Attachment> {
        use std::io::Read;
        request.validate().map_err(anyhow::Error::msg)?;
        let path = self.resolve(&request.path)?;
        ensure!(path.is_file(), "Attachment source must be a regular file");
        let mut file = std::fs::File::open(&path)?;
        ensure!(file.metadata()?.is_file(), "Attachment source must be a regular file");
        let name = path.file_name().context("Missing filename")?.to_string_lossy().into_owned();
        let mut bytes = Vec::new();
        (&mut file).take(12).read_to_end(&mut bytes)?;
        if !matches!(Attachment::from_bytes(name.clone(), &bytes), Ok(Attachment::Image { .. })) {
            return self.read(request);
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
        let directory = self.resolve(&path)?;
        ensure!(directory.is_dir(), "Context path must be a directory");
        let mut listing = WorkspaceFiles {
            path: path.clone(),
            entries: Vec::new(),
            truncated: false,
        };
        for (index, entry) in std::fs::read_dir(directory)?.enumerate() {
            if index == MAX_WORKSPACE_ENTRIES {
                listing.truncated = true;
                break;
            }
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name.eq_ignore_ascii_case(".git") || name.contains('\\') {
                continue;
            }
            let relative = if path.is_empty() { name } else { format!("{path}/{name}") };
            if (WorkspaceFilesRequest {
                path: relative.clone(),
            })
            .validate()
            .is_err()
            {
                continue;
            }
            // Canonical checks also exclude links into other workspaces or Git
            // metadata.
            let Ok(resolved) = self.resolve(&relative) else {
                continue;
            };
            if resolved.is_dir() || resolved.is_file() {
                listing.entries.push(WorkspaceFile {
                    path: relative,
                    directory: resolved.is_dir(),
                });
            }
        }
        listing.entries.sort_by(|a, b| b.directory.cmp(&a.directory).then_with(|| a.path.cmp(&b.path)));
        Ok(listing)
    }

    fn read(&self, request: &FileContextRequest) -> Result<Attachment> {
        request.validate().map_err(anyhow::Error::msg)?;
        let path = self.resolve(&request.path)?;
        ensure!(path.is_file(), "Context path must be a regular file");
        let file = std::fs::File::open(&path)?;
        let metadata = file.metadata()?;
        ensure!(metadata.is_file(), "Context path must be a regular file");
        ensure!(metadata.len() <= 16 * 1024 * 1024, "Context source exceeds 16 MiB");
        let mut reader = BufReader::new(file);
        let mut text = Vec::new();
        let mut line = Vec::new();
        let start = request.start_line.unwrap_or(1);
        let end = request.end_line.unwrap_or(usize::MAX);
        let mut count = 0;
        // Source size is checked above; bound each read as well if a file
        // grows.
        use std::io::Read;
        let mut reader = (&mut reader).take(16 * 1024 * 1024 + 1);
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
        let name = path.file_name().context("Missing filename")?.to_string_lossy().into_owned();
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
}

#[cfg(test)]
mod tests {
    use super::*;
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
            let link = root.join("escape");
            let output = std::process::Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "New-Item -ItemType Junction -Path $env:T3_CONTEXT_LINK -Target $env:T3_CONTEXT_TARGET -ErrorAction Stop | Out-Null",
                ])
                .env("T3_CONTEXT_LINK", &link)
                .env("T3_CONTEXT_TARGET", &outside)
                .output()
                .unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
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
