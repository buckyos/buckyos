use crate::AgentToolError;

pub fn u64_to_usize_arg(value: u64, key: &str) -> Result<usize, AgentToolError> {
    usize::try_from(value).map_err(|_| {
        AgentToolError::InvalidArgs(format!("`{key}` is too large for current platform"))
    })
}

