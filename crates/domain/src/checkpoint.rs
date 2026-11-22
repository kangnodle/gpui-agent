use serde::{Deserialize,
            Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitVersion {
    #[serde(default)]
    pub branch: Option<String>,
    pub head: Option<String>,
    pub index: String,
    pub worktree: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    pub label: String,
    pub created_at: i64,
    pub head: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum GitCommand {
    Inspect,
    PreviewPr {
        remote: String,
        base: String,
    },
    CreatePr {
        remote: String,
        base: String,
        title: String,
        body: String,
        draft: bool,
        push: bool,
        expected: GitVersion,
        expected_base: String,
        expected_branch: String,
        expected_repository: String,
    },
    Stage {
        path: String,
        staged: bool,
        expected: GitVersion,
    },
    Checkpoint {
        label: String,
    },
    PreviewRestore {
        checkpoint: String,
    },
    Restore {
        checkpoint: String,
        expected: GitVersion,
    },
    Commit {
        message: String,
        expected: GitVersion,
    },
    ProposeCommit {
        message: String,
        paths: Vec<String>,
    },
    CommitSelected {
        message: String,
        paths: Vec<String>,
        expected: GitVersion,
    },
}

impl GitCommand {
    pub fn validate(&self) -> Result<(), &'static str> {
        let text = match self {
            | Self::CreatePr {
                title, ..
            } => Some((title, 256)),
            | Self::Checkpoint {
                label,
            } => Some((label, 256)),
            | Self::Commit {
                message, ..
            }
            | Self::ProposeCommit {
                message, ..
            }
            | Self::CommitSelected {
                message, ..
            } => Some((message, 8192)),
            | _ => None,
        };
        if text.is_some_and(|(text, limit)| text.trim().is_empty() || text.len() > limit || text.contains('\0')) {
            return Err("Invalid checkpoint label or commit message");
        }
        if let Self::ProposeCommit {
            paths, ..
        }
        | Self::CommitSelected {
            paths, ..
        } = self
            && (paths.is_empty() || paths.len() > 4096 || paths.iter().any(|path| path.is_empty() || path.len() > 32768 || path.contains('\0')))
        {
            return Err("Select at least one valid changed file");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitOverview {
    pub workspace_id: i64,
    pub version: GitVersion,
    pub checkpoints: Vec<Checkpoint>,
    #[serde(default)]
    pub files: Vec<crate::ChangedFile>,
    pub diff: String,
    pub truncated: bool,
    pub backup: Option<String>,
    pub commit: Option<String>,
    #[serde(default)]
    pub pr: Option<PullRequest>,
    #[serde(default)]
    pub proposal: Option<CommitProposal>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitProposal {
    pub message: String,
    pub paths: Vec<String>,
    pub expected: GitVersion,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullRequest {
    pub repository: String,
    pub remote: String,
    pub branch: String,
    pub base: String,
    pub base_oid: String,
    pub commits: String,
    pub diff: String,
    pub truncated: bool,
    pub needs_push: bool,
    pub url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checkpoint_labels_keep_byte_limits_and_reject_blank_or_nul_input() {
        for (label, valid) in [
            ("한글".into(), true),
            ("x".repeat(256), true),
            ("x".repeat(257), false),
            (" \n".into(), false),
            ("a\0b".into(), false),
        ] {
            assert_eq!(
                GitCommand::Checkpoint {
                    label
                }
                .validate()
                .is_ok(),
                valid
            );
        }
        assert!(GitCommand::Inspect.validate().is_ok());
    }
}
