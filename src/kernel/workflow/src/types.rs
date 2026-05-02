//! Workflow 编译态 / 运行态共享的中间表示类型已经搬到 buckyos-api，给所有
//! 需要查看 / 渲染 workflow 编译产物或 Run Graph 的组件复用。这里保留一层
//! re-export 让 workflow crate 内部仍然通过 `crate::types::*` 访问。

pub use buckyos_api::workflow_types::*;

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
