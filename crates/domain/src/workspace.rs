use serde::{Deserialize,
            Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workspace {
    pub id: i64,
    pub project_id: i64,
    pub path: String,
    pub branch: Option<String>,
    pub main: bool,
    pub managed: bool,
    pub locked: bool,
    pub missing: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    #[serde(default)]
    pub verification: crate::ProjectVerification,
    pub id: i64,
    pub name: String,
    pub path: String,
    pub git: bool,
    pub workspaces: Vec<Workspace>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectCatalog {
    pub default_workspace_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registered_workspace_id: Option<i64>,
    pub projects: Vec<Project>,
}
impl ProjectCatalog {
    pub fn for_workspace(mut self, path: &str) -> Option<Self> {
        for project in &mut self.projects {
            project.workspaces.retain(|workspace| workspace.path == path && !workspace.missing);
        }
        self.projects.retain(|project| !project.workspaces.is_empty());
        self.default_workspace_id = Some(self.projects.first()?.workspaces.first()?.id);
        self.registered_workspace_id = None;
        Some(self)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangedFile {
    pub path: String,
    pub original_path: Option<String>,
    pub index_status: String,
    pub worktree_status: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitChanges {
    pub workspace_id: i64,
    pub branch: Option<String>,
    pub files: Vec<ChangedFile>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileDiff {
    pub workspace_id: i64,
    pub path: String,
    pub staged: String,
    pub unstaged: String,
    pub untracked: bool,
    pub truncated: bool,
}
