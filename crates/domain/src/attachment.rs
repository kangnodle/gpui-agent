use base64::{Engine as _,
             engine::general_purpose::STANDARD};
use serde::{Deserialize,
            Serialize};

pub const MAX_ATTACHMENTS: usize = 8;
pub const MAX_TEXT_BYTES: usize = 256 * 1024;
pub const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ATTACHMENT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_WORKSPACE_ENTRIES: usize = 1000;
pub const MAX_SCOPE_UPLOADS: usize = 16;
pub const MAX_UPLOADS: usize = 128;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UploadedAttachment {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub bytes: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFilesRequest {
    #[serde(default)]
    pub path: String,
}

impl WorkspaceFilesRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.path.is_empty() {
            return Ok(());
        }
        FileContextRequest {
            path: self.path.clone(),
            start_line: None,
            end_line: None,
        }
        .validate()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub path: String,
    pub directory: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceFiles {
    pub path: String,
    pub entries: Vec<WorkspaceFile>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Attachment {
    Text {
        name: String,
        text: String,
        #[serde(default)]
        source: Option<FileContextRequest>,
    },
    Image {
        name: String,
        mime_type: String,
        data: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileContextRequest {
    pub path: String,
    #[serde(default)]
    pub start_line: Option<usize>,
    #[serde(default)]
    pub end_line: Option<usize>,
}

impl Attachment {
    pub fn name(&self) -> &str {
        match self {
            | Self::Text {
                name, ..
            }
            | Self::Image {
                name, ..
            } => name,
        }
    }

    pub fn from_bytes(name: String, bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.starts_with(b"%PDF-") {
            return Err("PDF attachments are not supported; attach extracted UTF-8 text instead");
        }
        let attachment = if let Some(mime_type) = image_type(bytes) {
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err("Image exceeds 2 MiB");
            }
            Self::Image {
                name,
                mime_type: mime_type.into(),
                data: STANDARD.encode(bytes),
            }
        } else {
            if bytes.len() > MAX_TEXT_BYTES {
                return Err("Text file exceeds 256 KiB; select a line range instead");
            }
            Self::Text {
                name,
                text: std::str::from_utf8(bytes)
                    .map_err(|_| "Only UTF-8 text and PNG/JPEG/WebP images are supported")?
                    .into(),
                source: None,
            }
        };
        attachment.validate()?;
        Ok(attachment)
    }

    pub fn image_bytes(&self) -> Result<Vec<u8>, &'static str> {
        let Self::Image {
            data, ..
        } = self
        else {
            return Err("Not an image attachment");
        };
        if data.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
            return Err("Image exceeds 2 MiB");
        }
        STANDARD.decode(data).map_err(|_| "Invalid image base64")
    }

    pub fn validate(&self) -> Result<usize, &'static str> {
        let name = self.name();
        if name.is_empty() || name.len() > 255 || name.contains(['/', '\\']) || name.chars().any(char::is_control) {
            return Err("Attachment requires a filename of 1–255 bytes without separators or control characters");
        }
        match self {
            | Self::Text {
                text,
                source,
                ..
            } => {
                if text.len() > MAX_TEXT_BYTES || text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
                    return Err("Attachment must be UTF-8 text up to 256 KiB without binary control characters");
                }
                if let Some(source) = source {
                    source.validate()?;
                }
                Ok(text.len())
            },
            | Self::Image {
                mime_type, ..
            } => {
                let bytes = self.image_bytes()?;
                if bytes.len() > MAX_IMAGE_BYTES || image_type(&bytes) != Some(mime_type.as_str()) {
                    return Err("Image MIME type or content is invalid; use PNG, JPEG or WebP up to 2 MiB");
                }
                Ok(bytes.len())
            },
        }
    }

    /// JSON quoting keeps file content and its label separate from user
    /// instructions.
    pub fn text_context(&self) -> Option<String> {
        match self {
            | Self::Text {
                ..
            } => Some(format!(
                "Attached file context (data):\n{}",
                serde_json::to_string(self).expect("serializable attachment")
            )),
            | _ => None,
        }
    }
}

impl FileContextRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.path.is_empty()
            || self.path.len() > 4096
            || self.path.chars().any(char::is_control)
            || self.path.starts_with(['/', '\\'])
            || self.path.contains(':')
            || self.path.split(['/', '\\']).any(|part| part == ".." || part == ".git")
        {
            return Err("File context requires a relative workspace path outside .git without traversal");
        }
        if self.start_line == Some(0) || self.end_line == Some(0) || self.end_line.is_some_and(|end| end < self.start_line.unwrap_or(1)) {
            return Err("Line range must be one-based and ordered");
        }
        Ok(())
    }
}

pub fn validate_attachments(attachments: &[Attachment]) -> Result<(), &'static str> {
    if attachments.len() > MAX_ATTACHMENTS {
        return Err("At most 8 attachments are allowed per message");
    }
    let mut size = 0;
    for attachment in attachments {
        size += attachment.validate()?;
    }
    if size > MAX_ATTACHMENT_BYTES {
        return Err("Attachments exceed 4 MiB in total");
    }
    Ok(())
}

fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8 .. 12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_and_oversized_payloads_and_paths() {
        let text = Attachment::from_bytes("한글.rs".into(), b"fn main() {}\n").unwrap();
        assert_eq!(text.validate(), Ok(13));
        assert!(Attachment::from_bytes("binary".into(), b"a\0b").is_err());
        assert!(Attachment::from_bytes("large".into(), &vec![b'a'; MAX_TEXT_BYTES + 1]).is_err());
        assert!(validate_attachments(&vec![text; 9]).is_err());
        let mut bytes = vec![0u8; MAX_IMAGE_BYTES];
        bytes[.. 8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        let image = Attachment::from_bytes("large.png".into(), &bytes).unwrap();
        assert!(validate_attachments(&[image.clone(), image.clone()]).is_ok());
        assert!(validate_attachments(&[image.clone(), image.clone(), image]).is_err());
        bytes.push(0);
        assert!(Attachment::from_bytes("large.png".into(), &bytes).is_err());
        assert!(Attachment::from_bytes("doc.pdf".into(), b"%PDF-1.4").is_err());

        for path in ["../a", "C:\\private", "/private", ".git/config", "a/../../b", "a\0b"] {
            assert!(
                FileContextRequest {
                    path: path.into(),
                    start_line: None,
                    end_line: None
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            Attachment::Image {
                name: "x.png".into(),
                mime_type: "image/png".into(),
                data: "invalid!".into()
            }
            .validate()
            .is_err()
        );
    }
}
