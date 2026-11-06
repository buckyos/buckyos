//! Built-in FunctionObject implementations shipped with the workflow service.

pub mod media;
pub mod test_functions;

use crate::error::WorkflowResult;
use crate::function_registry::FunctionRegistry;

/// Registers every built-in function (test helpers + media).
pub fn register_builtin_functions(registry: &mut FunctionRegistry) -> WorkflowResult<()> {
    test_functions::register(registry)?;
    media::register(registry)?;
    Ok(())
}
