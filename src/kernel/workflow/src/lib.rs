mod adapters;
mod analysis;
mod compiler;
mod dispatcher;
mod dsl;
mod error;
mod executor_adapter;
mod object_store;
mod orchestrator;
mod runtime;
mod schema;
mod task_tracker;
mod types;

pub mod service_schemas {
    //! workflow 视角的服务 schema 定义。它们不是协议本身，是 DSL 作者写
    //! `executor: "service::xxx.yyy"` 时引擎用来约束输入输出的 workflow 子集。
    pub mod aicc {
        pub use crate::adapters::aicc::{
            aicc_method_schema, aicc_method_schemas, AiccAdapter, AiccMethodSchema,
            AICC_EXECUTOR_PREFIX,
        };
    }
}

pub use analysis::{analyze_workflow, AnalysisReport, AnalysisSeverity};
pub use buckyos_api::{
    FunctionObject, FunctionParamType, FunctionResultType, FunctionType, ResourceRequirements,
    ThunkExecutionResult, ThunkExecutionStatus, ThunkObject,
};
pub use compiler::{
    compile_workflow, CompileOutput, CompiledNode, CompiledWorkflow, WorkflowGraph,
};
pub use dispatcher::{InMemoryThunkDispatcher, ScheduledThunk, ThunkDispatcher};
pub use dsl::*;
pub use error::{WorkflowError, WorkflowResult};
pub use executor_adapter::{ExecutorAdapter, ExecutorRegistry, NamespaceAdapter};
pub use object_store::{InMemoryObjectStore, NamedStoreObjectStore, WorkflowObjectStore};
pub use orchestrator::WorkflowOrchestrator;
pub use runtime::*;
pub use task_tracker::{
    MapShardTaskView, NoopTaskTracker, RecordingTaskTracker, StepTaskView, TaskManagerTaskTracker,
    ThunkTaskView, WorkflowTaskTracker,
};
pub use types::*;
