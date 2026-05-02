use crate::analysis::{analyze_workflow, AnalysisIssue};
use crate::dsl::*;
use crate::error::{WorkflowError, WorkflowResult};
use crate::types::{AwaitKind, Expr, ExecutorRef, JoinStrategy, ValueTemplate};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledNode {
    pub id: String,
    pub name: String,
    pub expr: Expr,
    #[serde(default)]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub skippable: bool,
    #[serde(default)]
    pub idempotent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledWorkflow {
    pub schema_version: String,
    pub workflow_id: String,
    pub workflow_name: String,
    pub definition: WorkflowDefinition,
    pub nodes: BTreeMap<String, CompiledNode>,
    pub graph: WorkflowGraph,
    pub warnings: Vec<AnalysisIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileOutput {
    pub workflow: CompiledWorkflow,
    pub warnings: Vec<AnalysisIssue>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowGraph {
    pub start_nodes: Vec<String>,
    pub terminal_from: BTreeSet<String>,
    pub incoming: BTreeMap<String, BTreeSet<String>>,
    pub explicit_successors: BTreeMap<String, Vec<String>>,
    pub all_successors: BTreeMap<String, Vec<String>>,
    pub branch_targets: BTreeMap<String, BTreeMap<String, String>>,
    pub parallel_branches: BTreeMap<String, Vec<String>>,
    pub for_each_steps: BTreeMap<String, Vec<String>>,
}

impl WorkflowGraph {
    pub fn from_definition(
        workflow: &WorkflowDefinition,
        node_map: &BTreeMap<String, impl Clone>,
        report: &mut crate::analysis::AnalysisReport,
    ) -> Self {
        let mut graph = Self::default();

        for node_id in node_map.keys() {
            graph.incoming.entry(node_id.clone()).or_default();
            graph
                .explicit_successors
                .entry(node_id.clone())
                .or_default();
            graph.all_successors.entry(node_id.clone()).or_default();
        }

        for edge in &workflow.edges {
            if let Some(to) = &edge.to {
                graph
                    .incoming
                    .entry(to.clone())
                    .or_default()
                    .insert(edge.from.clone());
                graph
                    .explicit_successors
                    .entry(edge.from.clone())
                    .or_default()
                    .push(to.clone());
                graph
                    .all_successors
                    .entry(edge.from.clone())
                    .or_default()
                    .push(to.clone());
            } else {
                graph.terminal_from.insert(edge.from.clone());
            }
        }

        for node in &workflow.nodes {
            match node {
                ControlNodeDefinition::Branch(branch) => {
                    graph
                        .branch_targets
                        .insert(branch.id.clone(), branch.paths.clone());
                    for target in branch.paths.values() {
                        graph
                            .incoming
                            .entry(target.clone())
                            .or_default()
                            .insert(branch.id.clone());
                        graph
                            .all_successors
                            .entry(branch.id.clone())
                            .or_default()
                            .push(target.clone());
                    }
                }
                ControlNodeDefinition::Parallel(parallel) => {
                    graph
                        .parallel_branches
                        .insert(parallel.id.clone(), parallel.branches.clone());
                    for branch in &parallel.branches {
                        graph
                            .incoming
                            .entry(branch.clone())
                            .or_default()
                            .insert(parallel.id.clone());
                        graph
                            .all_successors
                            .entry(parallel.id.clone())
                            .or_default()
                            .push(branch.clone());
                    }
                }
                ControlNodeDefinition::ForEach(for_each) => {
                    graph
                        .for_each_steps
                        .insert(for_each.id.clone(), for_each.steps.clone());
                    if let Some(first) = for_each.steps.first() {
                        graph
                            .incoming
                            .entry(first.clone())
                            .or_default()
                            .insert(for_each.id.clone());
                        graph
                            .all_successors
                            .entry(for_each.id.clone())
                            .or_default()
                            .push(first.clone());
                    }
                    for window in for_each.steps.windows(2) {
                        let from = window[0].clone();
                        let to = window[1].clone();
                        graph
                            .incoming
                            .entry(to.clone())
                            .or_default()
                            .insert(from.clone());
                        graph.all_successors.entry(from).or_default().push(to);
                    }
                }
            }
        }

        graph.start_nodes = node_map
            .keys()
            .filter(|node_id| {
                graph
                    .incoming
                    .get(*node_id)
                    .map(|deps| deps.is_empty())
                    .unwrap_or(true)
            })
            .cloned()
            .collect();

        if graph.start_nodes.is_empty() {
            report.push(
                crate::analysis::AnalysisSeverity::Error,
                "missing_start_node",
                "workflow does not contain a start node",
                None,
            );
        }

        graph
    }

    pub fn successors(&self, node_id: &str) -> &[String] {
        self.all_successors
            .get(node_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn explicit_successors(&self, node_id: &str) -> &[String] {
        self.explicit_successors
            .get(node_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn dependencies(&self, node_id: &str) -> Vec<String> {
        self.incoming
            .get(node_id)
            .map(|items| items.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn is_upstream(&self, source: &str, target: &str) -> bool {
        if source == target {
            return false;
        }
        let mut stack = vec![source.to_string()];
        let mut visited = HashSet::new();
        while let Some(current) = stack.pop() {
            if !visited.insert(current.clone()) {
                continue;
            }
            for next in self.successors(&current) {
                if next == target {
                    return true;
                }
                stack.push(next.clone());
            }
            // Branches/steps inside a parallel or for_each scope have no edges
            // back out to whatever runs after the scope joins. Logically the
            // scope's explicit successors only run once every branch/step has
            // completed, so they are reachable from each member. We push only
            // the *explicit* successors (edges from `workflow.edges`) to avoid
            // making sibling branches upstream of each other.
            for (parallel_id, branches) in &self.parallel_branches {
                if branches.iter().any(|b| b == &current) {
                    for next in self.explicit_successors(parallel_id) {
                        if next == target {
                            return true;
                        }
                        stack.push(next.clone());
                    }
                }
            }
            for (for_each_id, steps) in &self.for_each_steps {
                if steps.iter().any(|s| s == &current) {
                    for next in self.explicit_successors(for_each_id) {
                        if next == target {
                            return true;
                        }
                        stack.push(next.clone());
                    }
                }
            }
        }
        false
    }

    pub fn reachable_from_starts(&self) -> HashSet<String> {
        let mut visited = HashSet::new();
        let mut stack = self.start_nodes.clone();
        while let Some(current) = stack.pop() {
            if !visited.insert(current.clone()) {
                continue;
            }
            for next in self.successors(&current) {
                stack.push(next.clone());
            }
        }
        visited
    }

    pub fn has_terminal_path(&self) -> bool {
        self.start_nodes
            .iter()
            .any(|start| self.reaches_terminal(start, &mut HashSet::new()))
    }

    fn reaches_terminal(&self, current: &str, visiting: &mut HashSet<String>) -> bool {
        if self.terminal_from.contains(current) {
            return true;
        }
        if !visiting.insert(current.to_string()) {
            return false;
        }
        for next in self.successors(current) {
            if self.reaches_terminal(next, visiting) {
                visiting.remove(current);
                return true;
            }
        }
        visiting.remove(current);
        false
    }

    pub fn strongly_connected_components(&self) -> Vec<Vec<String>> {
        let nodes = self.incoming.keys().cloned().collect::<Vec<_>>();
        let mut index = 0usize;
        let mut indices = HashMap::<String, usize>::new();
        let mut lowlinks = HashMap::<String, usize>::new();
        let mut stack = Vec::<String>::new();
        let mut on_stack = HashSet::<String>::new();
        let mut result = Vec::<Vec<String>>::new();

        fn strong_connect(
            node: String,
            graph: &WorkflowGraph,
            index: &mut usize,
            indices: &mut HashMap<String, usize>,
            lowlinks: &mut HashMap<String, usize>,
            stack: &mut Vec<String>,
            on_stack: &mut HashSet<String>,
            result: &mut Vec<Vec<String>>,
        ) {
            indices.insert(node.clone(), *index);
            lowlinks.insert(node.clone(), *index);
            *index += 1;
            stack.push(node.clone());
            on_stack.insert(node.clone());

            for next in graph.successors(&node) {
                if !indices.contains_key(next) {
                    strong_connect(
                        next.clone(),
                        graph,
                        index,
                        indices,
                        lowlinks,
                        stack,
                        on_stack,
                        result,
                    );
                    let next_lowlink = *lowlinks.get(next).unwrap_or(&0);
                    let node_lowlink = lowlinks.get_mut(&node).unwrap();
                    *node_lowlink = (*node_lowlink).min(next_lowlink);
                } else if on_stack.contains(next) {
                    let next_index = *indices.get(next).unwrap_or(&0);
                    let node_lowlink = lowlinks.get_mut(&node).unwrap();
                    *node_lowlink = (*node_lowlink).min(next_index);
                }
            }

            if indices.get(&node) == lowlinks.get(&node) {
                let mut component = Vec::new();
                while let Some(item) = stack.pop() {
                    on_stack.remove(&item);
                    component.push(item.clone());
                    if item == node {
                        break;
                    }
                }
                result.push(component);
            }
        }

        for node in nodes {
            if !indices.contains_key(&node) {
                strong_connect(
                    node,
                    self,
                    &mut index,
                    &mut indices,
                    &mut lowlinks,
                    &mut stack,
                    &mut on_stack,
                    &mut result,
                );
            }
        }

        result
    }

    pub fn downstream_from(&self, root: &str) -> Vec<String> {
        let mut result = Vec::new();
        let mut stack = self.successors(root).to_vec();
        let mut visited = HashSet::new();
        while let Some(node_id) = stack.pop() {
            if !visited.insert(node_id.clone()) {
                continue;
            }
            result.push(node_id.clone());
            for next in self.successors(&node_id) {
                stack.push(next.clone());
            }
        }
        result
    }
}

pub fn compile_workflow(definition: WorkflowDefinition) -> WorkflowResult<CompileOutput> {
    let (report, context) = analyze_workflow(&definition);
    if report.has_errors() {
        return Err(WorkflowError::Analysis(report));
    }

    let mut nodes = BTreeMap::new();
    for step in &definition.steps {
        let expr = compile_step(step, &definition)?;
        nodes.insert(
            step.id.clone(),
            CompiledNode {
                id: step.id.clone(),
                name: step.name.clone(),
                expr,
                output_schema: Some(step.output_schema.clone()),
                skippable: step.skippable,
                idempotent: step.idempotent,
            },
        );
    }

    for node in &definition.nodes {
        let (id, name, expr, output_schema) = compile_control_node(node, &definition)?;
        nodes.insert(
            id.clone(),
            CompiledNode {
                id,
                name,
                expr,
                output_schema: Some(output_schema),
                skippable: false,
                idempotent: true,
            },
        );
    }

    let compiled = CompiledWorkflow {
        schema_version: definition.schema_version.clone(),
        workflow_id: definition.id.clone(),
        workflow_name: definition.name.clone(),
        definition,
        nodes,
        graph: context.graph,
        warnings: report.warnings.clone(),
    };

    Ok(CompileOutput {
        workflow: compiled,
        warnings: report.warnings,
    })
}

fn compile_step(step: &StepDefinition, _workflow: &WorkflowDefinition) -> WorkflowResult<Expr> {
    match step.step_type {
        StepType::Autonomous => {
            let executor_str = step.executor.clone().ok_or_else(|| {
                WorkflowError::Serialization(format!(
                    "autonomous step `{}` requires executor",
                    step.id
                ))
            })?;
            let executor = ExecutorRef::parse(&executor_str).ok_or_else(|| {
                WorkflowError::UnknownExecutorNamespace {
                    node_id: step.id.clone(),
                    executor: executor_str.clone(),
                }
            })?;
            let params = match step.input.as_ref() {
                Some(Value::Object(map)) => map
                    .iter()
                    .map(|(key, value)| Ok((key.clone(), compile_value_template(value)?)))
                    .collect::<WorkflowResult<BTreeMap<_, _>>>()?,
                Some(other) => {
                    let mut params = BTreeMap::new();
                    params.insert("value".to_string(), compile_value_template(other)?);
                    params
                }
                None => BTreeMap::new(),
            };

            // 仅 `func::<objid>` 这类已绑定到 FunctionObject 的实际定义会有 fun_id；
            // 其余 (service:: / http:: / appservice:: / operator:: 以及未展开的
            // /agent/、/skill/、/tool/ 语义链接) 留空，由编排器在执行阶段决定走
            // adapter 还是先经 registry 展开。
            let fun_id = if executor.is_function_object() {
                Some(hash_executor(executor.as_str()))
            } else {
                None
            };

            Ok(Expr::Apply {
                executor,
                fun_id,
                params,
                output_mode: step.output_mode,
                idempotent: step.idempotent,
                step_type: step.step_type,
                guards: step.guards.clone().unwrap_or_default(),
            })
        }
        StepType::HumanConfirm => Ok(Expr::Await {
            kind: AwaitKind::Confirm,
            subject: step
                .subject_ref
                .as_deref()
                .and_then(crate::types::RefPath::parse),
            prompt: step.prompt.clone(),
            output_schema: step.output_schema.clone(),
        }),
        StepType::HumanRequired => Ok(Expr::Await {
            kind: AwaitKind::Required,
            subject: step
                .subject_ref
                .as_deref()
                .and_then(crate::types::RefPath::parse),
            prompt: step.prompt.clone(),
            output_schema: step.output_schema.clone(),
        }),
    }
}

fn compile_control_node(
    node: &ControlNodeDefinition,
    workflow: &WorkflowDefinition,
) -> WorkflowResult<(String, String, Expr, Value)> {
    match node {
        ControlNodeDefinition::Branch(branch) => Ok((
            branch.id.clone(),
            branch.id.clone(),
            Expr::Match {
                on: crate::types::RefPath::parse(&branch.on)
                    .ok_or_else(|| WorkflowError::InvalidReference(branch.on.clone()))?,
                cases: branch.paths.clone(),
                max_iterations: branch.max_iterations,
            },
            serde_json::json!({
                "type": "object",
                "properties": {
                    "branch": {
                        "type": "string",
                        "enum": branch.paths.keys().cloned().collect::<Vec<_>>()
                    }
                },
                "required": ["branch"]
            }),
        )),
        ControlNodeDefinition::Parallel(parallel) => {
            let join = match parallel.join {
                JoinMode::All => JoinStrategy::All,
                JoinMode::Any => JoinStrategy::Any,
                JoinMode::NOfM => JoinStrategy::NOfM(parallel.n.unwrap_or(1)),
            };
            let properties = parallel
                .branches
                .iter()
                .filter_map(|branch_id| {
                    workflow
                        .steps
                        .iter()
                        .find(|step| step.id == *branch_id)
                        .map(|step| (branch_id.clone(), step.output_schema.clone()))
                })
                .collect::<serde_json::Map<_, _>>();
            Ok((
                parallel.id.clone(),
                parallel.id.clone(),
                Expr::Par {
                    branches: parallel.branches.clone(),
                    join,
                },
                serde_json::json!({
                    "type": "object",
                    "properties": properties
                }),
            ))
        }
        ControlNodeDefinition::ForEach(for_each) => {
            let source_step = workflow.steps.iter().find(|step| {
                crate::types::RefPath::parse(&for_each.items)
                    .map(|reference| reference.node_id == step.id)
                    .unwrap_or(false)
            });
            let actual_concurrency = match source_step.map(|step| step.output_mode) {
                Some(OutputMode::FiniteSequential) => 1,
                _ => for_each.concurrency.max(1),
            };
            let last_schema = for_each
                .steps
                .last()
                .and_then(|step_id| workflow.steps.iter().find(|step| step.id == *step_id))
                .map(|step| step.output_schema.clone())
                .unwrap_or_else(|| serde_json::json!({}));
            Ok((
                for_each.id.clone(),
                for_each.id.clone(),
                Expr::Map {
                    collection: crate::types::RefPath::parse(&for_each.items)
                        .ok_or_else(|| WorkflowError::InvalidReference(for_each.items.clone()))?,
                    steps: for_each.steps.clone(),
                    max_items: for_each.max_items,
                    concurrency: for_each.concurrency,
                    actual_concurrency,
                },
                serde_json::json!({
                    "type": "array",
                    "items": last_schema
                }),
            ))
        }
    }
}

fn compile_value_template(value: &Value) -> WorkflowResult<ValueTemplate> {
    match value {
        Value::String(text) if text.starts_with("${") => {
            let reference = crate::types::RefPath::parse(text)
                .ok_or_else(|| WorkflowError::InvalidReference(text.clone()))?;
            Ok(ValueTemplate::Reference(reference))
        }
        Value::Array(items) => Ok(ValueTemplate::Array(
            items
                .iter()
                .map(compile_value_template)
                .collect::<WorkflowResult<Vec<_>>>()?,
        )),
        Value::Object(map) => Ok(ValueTemplate::Object(
            map.iter()
                .map(|(key, value)| Ok((key.clone(), compile_value_template(value)?)))
                .collect::<WorkflowResult<BTreeMap<_, _>>>()?,
        )),
        _ => Ok(ValueTemplate::Literal(value.clone())),
    }
}

fn hash_executor(executor: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(executor.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::compile_workflow;
    use crate::dsl::{
        BranchNodeDefinition, ControlNodeDefinition, EdgeDefinition, ForEachNodeDefinition,
        OutputMode, StepDefinition, StepType, WorkflowDefinition,
    };
    use crate::error::WorkflowError;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn compile_rejects_non_exhaustive_branch() {
        let workflow = WorkflowDefinition {
            schema_version: "0.2.0".to_string(),
            id: "wf-branch".to_string(),
            name: "branch".to_string(),
            description: None,
            trigger: json!({"type":"manual"}),
            steps: vec![
                StepDefinition {
                    id: "plan".to_string(),
                    name: "Plan".to_string(),
                    executor: Some("/agent/mia".to_string()),
                    step_type: StepType::Autonomous,
                    input: None,
                    input_schema: None,
                    output_schema: json!({
                        "type": "object",
                        "properties": {
                            "decision": { "type": "string", "enum": ["approved", "rejected"] }
                        },
                        "required": ["decision"]
                    }),
                    subject_ref: None,
                    prompt: None,
                    idempotent: true,
                    skippable: false,
                    output_mode: OutputMode::Single,
                    guards: None,
                },
                StepDefinition {
                    id: "done".to_string(),
                    name: "Done".to_string(),
                    executor: Some("/skill/finalize".to_string()),
                    step_type: StepType::Autonomous,
                    input: None,
                    input_schema: None,
                    output_schema: json!({"type":"object"}),
                    subject_ref: None,
                    prompt: None,
                    idempotent: true,
                    skippable: false,
                    output_mode: OutputMode::Single,
                    guards: None,
                },
            ],
            nodes: vec![ControlNodeDefinition::Branch(BranchNodeDefinition {
                id: "branch".to_string(),
                on: "${plan.output.decision}".to_string(),
                paths: [("approved".to_string(), "done".to_string())]
                    .into_iter()
                    .collect(),
                max_iterations: 1,
            })],
            edges: vec![
                EdgeDefinition {
                    from: "plan".to_string(),
                    to: Some("branch".to_string()),
                },
                EdgeDefinition {
                    from: "done".to_string(),
                    to: None,
                },
            ],
            guards: None,
            defs: BTreeMap::new(),
        };

        let err = compile_workflow(workflow).unwrap_err();
        match err {
            WorkflowError::Analysis(report) => {
                assert!(report
                    .errors
                    .iter()
                    .any(|issue| issue.code == "branch_not_exhaustive"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn compile_warns_on_serialized_foreach() {
        let workflow = WorkflowDefinition {
            schema_version: "0.2.0".to_string(),
            id: "wf-foreach".to_string(),
            name: "foreach".to_string(),
            description: None,
            trigger: json!({"type":"manual"}),
            steps: vec![
                StepDefinition {
                    id: "scan".to_string(),
                    name: "Scan".to_string(),
                    executor: Some("/skill/fs".to_string()),
                    step_type: StepType::Autonomous,
                    input: None,
                    input_schema: None,
                    output_schema: json!({
                        "type": "object",
                        "properties": {
                            "element_schema": { "type": "object" },
                            "total_count": { "type": "integer" }
                        }
                    }),
                    subject_ref: None,
                    prompt: None,
                    idempotent: true,
                    skippable: false,
                    output_mode: OutputMode::FiniteSequential,
                    guards: None,
                },
                StepDefinition {
                    id: "ingest".to_string(),
                    name: "Ingest".to_string(),
                    executor: Some("/skill/ingest".to_string()),
                    step_type: StepType::Autonomous,
                    input: None,
                    input_schema: None,
                    output_schema: json!({"type":"object"}),
                    subject_ref: None,
                    prompt: None,
                    idempotent: true,
                    skippable: false,
                    output_mode: OutputMode::Single,
                    guards: None,
                },
            ],
            nodes: vec![ControlNodeDefinition::ForEach(ForEachNodeDefinition {
                id: "loop".to_string(),
                items: "${scan.output}".to_string(),
                steps: vec!["ingest".to_string()],
                max_items: 10,
                concurrency: 5,
            })],
            edges: vec![
                EdgeDefinition {
                    from: "scan".to_string(),
                    to: Some("loop".to_string()),
                },
                EdgeDefinition {
                    from: "ingest".to_string(),
                    to: None,
                },
            ],
            guards: None,
            defs: BTreeMap::new(),
        };

        let output = compile_workflow(workflow).unwrap();
        assert!(output
            .warnings
            .iter()
            .any(|issue| issue.code == "for_each_serialized"));
    }
}
