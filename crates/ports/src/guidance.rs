use anyhow::Result;
use gpui_agent_domain::{GuidanceFile,
                        GuidanceFileCatalog,
                        GuidanceSnapshot,
                        GuidanceWriteRequest,
                        SkillResource};

pub trait GuidanceSource: Send + Sync {
    fn import_skill(&self, request: &gpui_agent_domain::SkillImportRequest) -> Result<GuidanceFile>;
    fn snapshot(&self) -> Result<GuidanceSnapshot>;
    fn read_skill(&self, id: &str, resource: &str) -> Result<SkillResource>;
    fn set_skill_enabled(&self, id: &str, enabled: bool) -> Result<()>;
    fn files(&self) -> Result<GuidanceFileCatalog>;
    fn read_file(&self, id: &str) -> Result<GuidanceFile>;
    fn write_file(&self, request: &GuidanceWriteRequest) -> Result<GuidanceFile>;
}
