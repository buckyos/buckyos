use std::path::{Path, PathBuf};

use crate::agent_tool::{ToolError, ToolManager};
use crate::workspace::{AgentWorkshop, AgentWorkshopConfig};

#[derive(Clone, Debug)]
pub struct AgentEnvironment {
    workshop: AgentWorkshop,
}

impl AgentEnvironment {
    pub async fn new(workspace_root: impl Into<PathBuf>) -> Result<Self, ToolError> {
        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(workspace_root)).await?;
        Ok(Self { workshop })
    }

    pub fn workspace_root(&self) -> &Path {
        self.workshop.workspace_root()
    }

    pub fn register_workshop_tools(&self, tool_mgr: &ToolManager) -> Result<(), ToolError> {
        self.workshop.register_tools(tool_mgr)
    }

    // Backward compatibility for old call sites.
    pub fn register_basic_workshop_tools(&self, tool_mgr: &ToolManager) -> Result<(), ToolError> {
        self.register_workshop_tools(tool_mgr)
    }
}
