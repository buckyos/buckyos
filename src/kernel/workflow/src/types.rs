use crate::dsl::{GuardConfig, OutputMode, RetryFallback, RetryGuard, StepType};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RefPath {
    pub node_id: String,
    pub field_path: Vec<String>,
}

impl RefPath {
    pub fn parse(raw: &str) -> Option<Self> {
        if !raw.starts_with("${") || !raw.ends_with('}') {
            return None;
        }
        let inner = &raw[2..raw.len() - 1];
        let mut parts = inner.split('.');
        let node_id = parts.next()?.to_string();
        match parts.next() {
            Some("output") => {}
            _ => return None,
        }
        let field_path = parts.map(|part| part.to_string()).collect::<Vec<_>>();
        if node_id.is_empty() {
            return None;
        }
        Some(Self {
            node_id,
            field_path,
        })
    }

    pub fn as_string(&self) -> String {
        if self.field_path.is_empty() {
            format!("${{{}.output}}", self.node_id)
        } else {
            format!("${{{}.output.{}}}", self.node_id, self.field_path.join("."))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValueTemplate {
    Literal(Value),
    Reference(RefPath),
    Array(Vec<ValueTemplate>),
    Object(BTreeMap<String, ValueTemplate>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AwaitKind {
    Confirm,
    Required,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JoinStrategy {
    All,
    Any,
    NOfM(u32),
}

/// DSL `executor` 字段的语义分类。
///
/// - `Actual` 表示已经定位到具体执行入口的实际 executor 定义，使用 `namespace::name`
///   的形式（例如 `service::aicc.complete`、`http::file-classifier.classify`、
///   `appservice::media-tools.extract_metadata`、`operator::json.pick`、
///   `func::<function_object_id>`）。这类引用可以直接交给对应的编排器侧 adapter
///   或调度器执行。
/// - `SemanticPath` 表示能力级别的语义链接，使用 `/agent/<name>` /
///   `/skill/<name>` / `/tool/<name>` 这样的路径形式。它描述“这一步需要什么能力”，
///   运行前需要通过 executor registry 展开到一个 `Actual` 形式的实际定义。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutorRef {
    Actual(String),
    SemanticPath(String),
}

impl ExecutorRef {
    /// 解析 DSL 中 `executor` 字段的字符串到分类。
    ///
    /// 规则：
    /// - 以 `/` 开头视作 `SemanticPath`（如 `/agent/mia`、`/skill/fs-scanner`）。
    /// - 形如 `<namespace>::<rest>` 视作 `Actual`，其中 `namespace` 必须是已知的实际
    ///   executor 命名空间（service / http / appservice / operator / func）。
    /// - 其他写法暂不属于任何已知 namespace，返回 `None` 由调用者按上下文报错。
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.starts_with('/') {
            return Some(Self::SemanticPath(trimmed.to_string()));
        }
        if let Some((ns, _)) = trimmed.split_once("::") {
            match ns {
                "service" | "http" | "appservice" | "operator" | "func" => {
                    return Some(Self::Actual(trimmed.to_string()));
                }
                _ => return None,
            }
        }
        None
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Actual(value) | Self::SemanticPath(value) => value.as_str(),
        }
    }

    pub fn namespace(&self) -> Option<&str> {
        match self {
            Self::Actual(value) => value.split_once("::").map(|(ns, _)| ns),
            Self::SemanticPath(value) => value.trim_start_matches('/').split('/').next(),
        }
    }

    pub fn is_function_object(&self) -> bool {
        matches!(self, Self::Actual(value) if value.starts_with("func::"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Expr {
    Apply {
        executor: ExecutorRef,
        /// FunctionObject 的内容寻址标识。仅在 executor 解析为 FunctionObject
        /// （`func::<objid>` 或 registry 展开后的 `func::...`）时填写；其余情况
        /// （编排器侧 adapter 直接执行、未展开的语义链接）保持为 `None`。
        fun_id: Option<String>,
        params: BTreeMap<String, ValueTemplate>,
        output_mode: OutputMode,
        idempotent: bool,
        step_type: StepType,
        guards: GuardConfig,
    },
    Match {
        on: RefPath,
        cases: BTreeMap<String, String>,
        max_iterations: u32,
    },
    Par {
        branches: Vec<String>,
        join: JoinStrategy,
    },
    Map {
        collection: RefPath,
        steps: Vec<String>,
        max_items: u32,
        concurrency: u32,
        actual_concurrency: u32,
    },
    Await {
        kind: AwaitKind,
        subject: Option<RefPath>,
        prompt: Option<String>,
        output_schema: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub fallback: RetryFallback,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            fallback: RetryFallback::Human,
        }
    }
}

impl From<Option<RetryGuard>> for RetryPolicy {
    fn from(value: Option<RetryGuard>) -> Self {
        let Some(value) = value else {
            return Self::default();
        };
        Self {
            max_attempts: value.max_attempts.max(1),
            fallback: value.fallback.unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ExecutorRef;

    #[test]
    fn parse_classifies_actual_namespaces() {
        for raw in [
            "service::aicc.complete",
            "http::file-classifier.classify",
            "appservice::media-tools.extract_metadata",
            "operator::json.pick",
            "func::abcdef0123456789",
        ] {
            assert_eq!(
                ExecutorRef::parse(raw),
                Some(ExecutorRef::Actual(raw.to_string())),
                "expected Actual for `{}`",
                raw
            );
        }
    }

    #[test]
    fn parse_classifies_semantic_paths() {
        for raw in ["/agent/mia", "/skill/fs-scanner", "/tool/image-normalizer"] {
            assert_eq!(
                ExecutorRef::parse(raw),
                Some(ExecutorRef::SemanticPath(raw.to_string())),
                "expected SemanticPath for `{}`",
                raw
            );
        }
    }

    #[test]
    fn parse_rejects_unknown_namespace() {
        assert_eq!(ExecutorRef::parse("skill/fs.scan"), None);
        assert_eq!(ExecutorRef::parse("agent/mia"), None);
        assert_eq!(ExecutorRef::parse("other::foo"), None);
        assert_eq!(ExecutorRef::parse(""), None);
    }

    #[test]
    fn is_function_object_only_for_func_actual() {
        assert!(ExecutorRef::parse("func::xyz").unwrap().is_function_object());
        assert!(!ExecutorRef::parse("service::a.b")
            .unwrap()
            .is_function_object());
        assert!(!ExecutorRef::parse("/skill/fs").unwrap().is_function_object());
    }
}
