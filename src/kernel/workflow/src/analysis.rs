use crate::compiler::WorkflowGraph;
use crate::dsl::*;
use crate::schema::{
    schema_accepts_null, schema_at_path, schema_enum_values, schemas_compatible, schemas_equal,
};
use crate::types::RefPath;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisIssue {
    pub severity: AnalysisSeverity,
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub node_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnalysisReport {
    #[serde(default)]
    pub errors: Vec<AnalysisIssue>,
    #[serde(default)]
    pub warnings: Vec<AnalysisIssue>,
}

impl AnalysisReport {
    pub fn push(
        &mut self,
        severity: AnalysisSeverity,
        code: impl Into<String>,
        message: impl Into<String>,
        node_id: Option<String>,
    ) {
        let issue = AnalysisIssue {
            severity,
            code: code.into(),
            message: message.into(),
            node_id,
        };
        match severity {
            AnalysisSeverity::Error => self.errors.push(issue),
            AnalysisSeverity::Warning => self.warnings.push(issue),
        }
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

impl fmt::Display for AnalysisReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let error_count = self.errors.len();
        let warning_count = self.warnings.len();
        write!(f, "{} errors, {} warnings", error_count, warning_count)
    }
}

#[derive(Debug, Clone)]
pub struct AnalysisContext {
    pub graph: WorkflowGraph,
    pub output_schemas: BTreeMap<String, Value>,
    pub parsed_refs: BTreeMap<String, Vec<ParsedReference>>,
}

#[derive(Debug, Clone)]
pub struct ParsedReference {
    pub owner_node_id: String,
    pub json_path: Vec<String>,
    pub reference: RefPath,
}

#[derive(Debug, Clone)]
enum NodeEntry<'a> {
    Step(&'a StepDefinition),
    Control(&'a ControlNodeDefinition),
}

pub fn analyze_workflow(workflow: &WorkflowDefinition) -> (AnalysisReport, AnalysisContext) {
    let mut report = AnalysisReport::default();

    if workflow.schema_version.trim().is_empty() {
        report.push(
            AnalysisSeverity::Error,
            "schema_version",
            "schema_version is required",
            None,
        );
    }

    let mut node_map: BTreeMap<String, NodeEntry<'_>> = BTreeMap::new();
    for step in &workflow.steps {
        if node_map
            .insert(step.id.clone(), NodeEntry::Step(step))
            .is_some()
        {
            report.push(
                AnalysisSeverity::Error,
                "duplicate_node_id",
                format!("duplicate node id `{}`", step.id),
                Some(step.id.clone()),
            );
        }
    }
    for node in &workflow.nodes {
        if node_map
            .insert(node.id().to_string(), NodeEntry::Control(node))
            .is_some()
        {
            report.push(
                AnalysisSeverity::Error,
                "duplicate_node_id",
                format!("duplicate node id `{}`", node.id()),
                Some(node.id().to_string()),
            );
        }
    }

    let graph = WorkflowGraph::from_definition(workflow, &node_map, &mut report);
    let mut output_schemas = build_output_schemas(workflow, &node_map, &mut report);
    let parsed_refs = collect_references(workflow, &mut report);

    validate_edges(workflow, &node_map, &mut report);
    validate_references(
        workflow,
        &node_map,
        &graph,
        &parsed_refs,
        &mut output_schemas,
        &mut report,
    );
    validate_reachability(&graph, &node_map, &mut report);
    validate_termination(&graph, &mut report);
    validate_cycles(workflow, &graph, &node_map, &mut report);
    validate_subject_refs(workflow, &output_schemas, &mut report);
    validate_branch_exhaustiveness(workflow, &output_schemas, &mut report);
    validate_skip_compatibility(workflow, &parsed_refs, &output_schemas, &mut report);
    validate_budget(workflow, &mut report);
    validate_output_modes(workflow, &mut report);
    validate_for_each(workflow, &node_map, &output_schemas, &mut report);

    (
        report,
        AnalysisContext {
            graph,
            output_schemas,
            parsed_refs,
        },
    )
}

fn validate_edges(
    workflow: &WorkflowDefinition,
    node_map: &BTreeMap<String, NodeEntry<'_>>,
    report: &mut AnalysisReport,
) {
    for edge in &workflow.edges {
        if !node_map.contains_key(&edge.from) {
            report.push(
                AnalysisSeverity::Error,
                "edge_unknown_from",
                format!("edge.from `{}` does not exist", edge.from),
                Some(edge.from.clone()),
            );
        }
        if let Some(to) = &edge.to {
            if !node_map.contains_key(to) {
                report.push(
                    AnalysisSeverity::Error,
                    "edge_unknown_to",
                    format!("edge.to `{}` does not exist", to),
                    Some(edge.from.clone()),
                );
            }
        }
    }
}

fn build_output_schemas(
    workflow: &WorkflowDefinition,
    node_map: &BTreeMap<String, NodeEntry<'_>>,
    report: &mut AnalysisReport,
) -> BTreeMap<String, Value> {
    let mut result = BTreeMap::new();
    for step in &workflow.steps {
        result.insert(step.id.clone(), step.output_schema.clone());
    }

    for node in &workflow.nodes {
        match node {
            ControlNodeDefinition::Branch(branch) => {
                result.insert(
                    branch.id.clone(),
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
                );
            }
            ControlNodeDefinition::Parallel(parallel) => {
                let mut properties = serde_json::Map::new();
                for branch_id in &parallel.branches {
                    if let Some(NodeEntry::Step(step)) = node_map.get(branch_id) {
                        properties.insert(branch_id.clone(), step.output_schema.clone());
                    } else {
                        report.push(
                            AnalysisSeverity::Error,
                            "parallel_branch_unknown",
                            format!("parallel branch `{}` must reference a step", branch_id),
                            Some(parallel.id.clone()),
                        );
                    }
                }
                result.insert(
                    parallel.id.clone(),
                    Value::Object(
                        [
                            ("type".to_string(), Value::String("object".to_string())),
                            ("properties".to_string(), Value::Object(properties)),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                );
            }
            ControlNodeDefinition::ForEach(for_each) => {
                let last_schema = for_each
                    .steps
                    .last()
                    .and_then(|step_id| result.get(step_id))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                result.insert(
                    for_each.id.clone(),
                    serde_json::json!({
                        "type": "array",
                        "items": last_schema
                    }),
                );
            }
        }
    }

    result
}

fn collect_references(
    workflow: &WorkflowDefinition,
    report: &mut AnalysisReport,
) -> BTreeMap<String, Vec<ParsedReference>> {
    let mut result = BTreeMap::new();

    for step in &workflow.steps {
        let mut refs = Vec::new();
        if let Some(input) = &step.input {
            collect_refs_from_value(step.id.as_str(), vec![], input, &mut refs, report);
        }
        if let Some(subject_ref) = &step.subject_ref {
            match RefPath::parse(subject_ref) {
                Some(reference) => refs.push(ParsedReference {
                    owner_node_id: step.id.clone(),
                    json_path: vec!["subject_ref".to_string()],
                    reference,
                }),
                None => report.push(
                    AnalysisSeverity::Error,
                    "invalid_subject_ref",
                    format!("invalid subject_ref `{}`", subject_ref),
                    Some(step.id.clone()),
                ),
            }
        }
        result.insert(step.id.clone(), refs);
    }

    for node in &workflow.nodes {
        let refs = match node {
            ControlNodeDefinition::Branch(branch) => match RefPath::parse(&branch.on) {
                Some(reference) => vec![ParsedReference {
                    owner_node_id: branch.id.clone(),
                    json_path: vec!["on".to_string()],
                    reference,
                }],
                None => {
                    report.push(
                        AnalysisSeverity::Error,
                        "invalid_branch_ref",
                        format!("invalid branch.on `{}`", branch.on),
                        Some(branch.id.clone()),
                    );
                    vec![]
                }
            },
            ControlNodeDefinition::ForEach(for_each) => match RefPath::parse(&for_each.items) {
                Some(reference) => vec![ParsedReference {
                    owner_node_id: for_each.id.clone(),
                    json_path: vec!["items".to_string()],
                    reference,
                }],
                None => {
                    report.push(
                        AnalysisSeverity::Error,
                        "invalid_for_each_ref",
                        format!("invalid for_each.items `{}`", for_each.items),
                        Some(for_each.id.clone()),
                    );
                    vec![]
                }
            },
            ControlNodeDefinition::Parallel(parallel) => {
                result.insert(parallel.id.clone(), vec![]);
                continue;
            }
        };
        result.insert(node.id().to_string(), refs);
    }

    result
}

fn collect_refs_from_value(
    owner_node_id: &str,
    path: Vec<String>,
    value: &Value,
    output: &mut Vec<ParsedReference>,
    report: &mut AnalysisReport,
) {
    match value {
        Value::String(text) => {
            if text.starts_with("${") {
                match RefPath::parse(text) {
                    Some(reference) => output.push(ParsedReference {
                        owner_node_id: owner_node_id.to_string(),
                        json_path: path,
                        reference,
                    }),
                    None => report.push(
                        AnalysisSeverity::Error,
                        "invalid_reference",
                        format!("invalid reference `{}`", text),
                        Some(owner_node_id.to_string()),
                    ),
                }
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let mut next = path.clone();
                next.push(index.to_string());
                collect_refs_from_value(owner_node_id, next, item, output, report);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                let mut next = path.clone();
                next.push(key.clone());
                collect_refs_from_value(owner_node_id, next, item, output, report);
            }
        }
        _ => {}
    }
}

fn validate_references(
    workflow: &WorkflowDefinition,
    node_map: &BTreeMap<String, NodeEntry<'_>>,
    graph: &WorkflowGraph,
    parsed_refs: &BTreeMap<String, Vec<ParsedReference>>,
    output_schemas: &mut BTreeMap<String, Value>,
    report: &mut AnalysisReport,
) {
    for (owner_id, refs) in parsed_refs {
        for parsed in refs {
            let reference = &parsed.reference;
            if !node_map.contains_key(&reference.node_id) {
                report.push(
                    AnalysisSeverity::Error,
                    "reference_unknown_node",
                    format!("reference points to unknown node `{}`", reference.node_id),
                    Some(owner_id.clone()),
                );
                continue;
            }

            if !graph.is_upstream(&reference.node_id, owner_id) && reference.node_id != *owner_id {
                report.push(
                    AnalysisSeverity::Error,
                    "reference_not_upstream",
                    format!(
                        "reference `{}` is not upstream of `{}`",
                        reference.as_string(),
                        owner_id
                    ),
                    Some(owner_id.clone()),
                );
            }

            let Some(referenced_schema) = output_schemas.get(&reference.node_id) else {
                continue;
            };
            if schema_at_path(referenced_schema, &reference.field_path, &workflow.defs).is_none() {
                report.push(
                    AnalysisSeverity::Error,
                    "reference_unknown_field",
                    format!(
                        "reference path `{}` does not exist in `{}` output_schema",
                        reference.field_path.join("."),
                        reference.node_id
                    ),
                    Some(owner_id.clone()),
                );
            }
        }
    }

    for step in &workflow.steps {
        let Some(input_schema) = &step.input_schema else {
            continue;
        };
        let refs = parsed_refs.get(&step.id).cloned().unwrap_or_default();
        for parsed in refs
            .into_iter()
            .filter(|parsed| parsed.json_path.first().map(String::as_str) != Some("subject_ref"))
        {
            let Some(source_schema) =
                output_schemas
                    .get(&parsed.reference.node_id)
                    .and_then(|schema| {
                        schema_at_path(schema, &parsed.reference.field_path, &workflow.defs)
                    })
            else {
                continue;
            };

            if let Some(expected_schema) =
                schema_at_path(input_schema, &parsed.json_path, &workflow.defs)
            {
                if !schemas_compatible(&source_schema, &expected_schema, &workflow.defs) {
                    report.push(
                        AnalysisSeverity::Error,
                        "input_schema_mismatch",
                        format!(
                            "reference `{}` is incompatible with input_schema path `{}`",
                            parsed.reference.as_string(),
                            parsed.json_path.join(".")
                        ),
                        Some(step.id.clone()),
                    );
                }
            }
        }
    }
}

fn validate_reachability(
    graph: &WorkflowGraph,
    node_map: &BTreeMap<String, NodeEntry<'_>>,
    report: &mut AnalysisReport,
) {
    let reachable = graph.reachable_from_starts();
    for node_id in node_map.keys() {
        if !reachable.contains(node_id) {
            report.push(
                AnalysisSeverity::Error,
                "unreachable_node",
                format!("node `{}` is not reachable from any start node", node_id),
                Some(node_id.clone()),
            );
        }
    }
}

fn validate_termination(graph: &WorkflowGraph, report: &mut AnalysisReport) {
    if !graph.has_terminal_path() {
        report.push(
            AnalysisSeverity::Error,
            "missing_terminal_path",
            "workflow does not contain a path to any terminal edge",
            None,
        );
    }
}

fn validate_cycles(
    workflow: &WorkflowDefinition,
    graph: &WorkflowGraph,
    node_map: &BTreeMap<String, NodeEntry<'_>>,
    report: &mut AnalysisReport,
) {
    for component in graph.strongly_connected_components() {
        let is_cycle = component.len() > 1
            || component
                .iter()
                .any(|node_id| graph.successors(node_id).contains(node_id));
        if !is_cycle {
            continue;
        }

        let mut bounded = false;
        for node_id in &component {
            if let Some(NodeEntry::Control(ControlNodeDefinition::Branch(branch))) =
                node_map.get(node_id)
            {
                if branch.max_iterations > 0 {
                    bounded = true;
                    break;
                }
            }
        }
        if !bounded {
            report.push(
                AnalysisSeverity::Error,
                "unbounded_cycle",
                format!(
                    "cycle {:?} does not include a branch with max_iterations",
                    component
                ),
                component.first().cloned(),
            );
        }
    }

    if workflow.steps.is_empty() {
        report.push(
            AnalysisSeverity::Error,
            "missing_steps",
            "workflow must contain at least one step",
            None,
        );
    }
}

fn validate_subject_refs(
    workflow: &WorkflowDefinition,
    output_schemas: &BTreeMap<String, Value>,
    report: &mut AnalysisReport,
) {
    for step in &workflow.steps {
        if step.step_type != StepType::HumanConfirm {
            continue;
        }
        let Some(subject_ref) = &step.subject_ref else {
            report.push(
                AnalysisSeverity::Error,
                "subject_ref_required",
                "human_confirm step requires subject_ref",
                Some(step.id.clone()),
            );
            continue;
        };
        let Some(reference) = RefPath::parse(subject_ref) else {
            continue;
        };
        let Some(subject_schema) = output_schemas
            .get(&reference.node_id)
            .and_then(|schema| schema_at_path(schema, &reference.field_path, &workflow.defs))
        else {
            continue;
        };
        let Some(final_subject_schema) = schema_at_path(
            &step.output_schema,
            &[String::from("final_subject")],
            &workflow.defs,
        ) else {
            report.push(
                AnalysisSeverity::Error,
                "missing_final_subject",
                "human_confirm output_schema must contain final_subject",
                Some(step.id.clone()),
            );
            continue;
        };
        if !schemas_equal(&subject_schema, &final_subject_schema, &workflow.defs) {
            report.push(
                AnalysisSeverity::Error,
                "subject_schema_mismatch",
                "subject_ref schema does not match output.final_subject schema",
                Some(step.id.clone()),
            );
        }
    }
}

fn validate_branch_exhaustiveness(
    workflow: &WorkflowDefinition,
    output_schemas: &BTreeMap<String, Value>,
    report: &mut AnalysisReport,
) {
    for node in &workflow.nodes {
        let ControlNodeDefinition::Branch(branch) = node else {
            continue;
        };
        let Some(reference) = RefPath::parse(&branch.on) else {
            continue;
        };
        let Some(source_schema) = output_schemas
            .get(&reference.node_id)
            .and_then(|schema| schema_at_path(schema, &reference.field_path, &workflow.defs))
        else {
            continue;
        };
        if let Some(enum_values) = schema_enum_values(&source_schema, &workflow.defs) {
            let path_values = branch.paths.keys().cloned().collect::<HashSet<_>>();
            let missing = enum_values
                .into_iter()
                .filter(|value| !path_values.contains(value))
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                report.push(
                    AnalysisSeverity::Error,
                    "branch_not_exhaustive",
                    format!("branch is missing paths for {:?}", missing),
                    Some(branch.id.clone()),
                );
            }
        }
    }
}

fn validate_skip_compatibility(
    workflow: &WorkflowDefinition,
    parsed_refs: &BTreeMap<String, Vec<ParsedReference>>,
    output_schemas: &BTreeMap<String, Value>,
    report: &mut AnalysisReport,
) {
    let skippable = workflow
        .steps
        .iter()
        .filter(|step| step.skippable)
        .map(|step| step.id.clone())
        .collect::<HashSet<_>>();

    for step in &workflow.steps {
        for parsed in parsed_refs.get(&step.id).cloned().unwrap_or_default() {
            if !skippable.contains(&parsed.reference.node_id) {
                continue;
            }
            let Some(input_schema) = &step.input_schema else {
                report.push(
                    AnalysisSeverity::Warning,
                    "skip_compatibility_unknown",
                    format!(
                        "step `{}` references skippable node `{}` but does not declare input_schema",
                        step.id, parsed.reference.node_id
                    ),
                    Some(step.id.clone()),
                );
                continue;
            };
            let Some(expected_schema) =
                schema_at_path(input_schema, &parsed.json_path, &workflow.defs)
            else {
                continue;
            };
            if !schema_accepts_null(&expected_schema, &workflow.defs) {
                report.push(
                    AnalysisSeverity::Error,
                    "skip_incompatible",
                    format!(
                        "step `{}` references skippable node `{}` but input path `{}` does not accept null",
                        step.id,
                        parsed.reference.node_id,
                        parsed.json_path.join(".")
                    ),
                    Some(step.id.clone()),
                );
            }
        }
    }

    let _ = output_schemas;
}

fn validate_budget(workflow: &WorkflowDefinition, report: &mut AnalysisReport) {
    let Some(global_budget) = workflow
        .guards
        .as_ref()
        .and_then(|guards| guards.budget.as_ref())
        .and_then(|budget| budget.max_cost_usdb)
    else {
        return;
    };

    let total_step_budget = workflow
        .steps
        .iter()
        .filter_map(|step| {
            step.guards
                .as_ref()
                .and_then(|guards| guards.max_cost_usdb)
                .or_else(|| {
                    step.guards
                        .as_ref()
                        .and_then(|guards| guards.budget.as_ref())
                        .and_then(|budget| budget.max_cost_usdb)
                })
        })
        .sum::<f64>();

    if total_step_budget > global_budget {
        report.push(
            AnalysisSeverity::Warning,
            "budget_overcommit",
            format!(
                "sum of step budgets ({total_step_budget}) exceeds global budget ({global_budget})"
            ),
            None,
        );
    }
}

fn validate_output_modes(workflow: &WorkflowDefinition, report: &mut AnalysisReport) {
    for step in &workflow.steps {
        if matches!(
            step.output_mode,
            OutputMode::FiniteSeekable | OutputMode::FiniteSequential
        ) {
            let has_element_schema = schema_at_path(
                &step.output_schema,
                &[String::from("element_schema")],
                &workflow.defs,
            )
            .is_some();
            if !has_element_schema {
                report.push(
                    AnalysisSeverity::Error,
                    "output_mode_schema",
                    "finite output_mode requires output_schema.properties.element_schema",
                    Some(step.id.clone()),
                );
            }
        }
    }
}

fn validate_for_each(
    workflow: &WorkflowDefinition,
    node_map: &BTreeMap<String, NodeEntry<'_>>,
    output_schemas: &BTreeMap<String, Value>,
    report: &mut AnalysisReport,
) {
    let output_modes = workflow
        .steps
        .iter()
        .map(|step| (step.id.clone(), step.output_mode))
        .collect::<HashMap<_, _>>();

    for node in &workflow.nodes {
        let ControlNodeDefinition::ForEach(for_each) = node else {
            continue;
        };
        let Some(reference) = RefPath::parse(&for_each.items) else {
            continue;
        };
        let mode = output_modes
            .get(&reference.node_id)
            .copied()
            .unwrap_or(OutputMode::Single);
        if mode == OutputMode::Single {
            report.push(
                AnalysisSeverity::Error,
                "for_each_single_input",
                "for_each.items cannot reference a single output",
                Some(for_each.id.clone()),
            );
        }
        if mode == OutputMode::FiniteSequential && for_each.concurrency > 1 {
            report.push(
                AnalysisSeverity::Warning,
                "for_each_serialized",
                format!(
                    "for_each `{}` requested concurrency {} but upstream is finite_sequential; actual concurrency will be 1",
                    for_each.id, for_each.concurrency
                ),
                Some(for_each.id.clone()),
            );
        }
        if for_each.steps.is_empty() {
            report.push(
                AnalysisSeverity::Error,
                "for_each_missing_steps",
                "for_each requires at least one internal step",
                Some(for_each.id.clone()),
            );
        }
        for step_id in &for_each.steps {
            if !matches!(node_map.get(step_id), Some(NodeEntry::Step(_))) {
                report.push(
                    AnalysisSeverity::Error,
                    "for_each_step_unknown",
                    format!("for_each internal step `{}` does not exist", step_id),
                    Some(for_each.id.clone()),
                );
            }
        }
        let _ = output_schemas;
    }
}
