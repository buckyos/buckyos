use crate::{
    validate_exact_model_name, AiccError, AiccErrorCode, DecisionEvaluateRequest, ModelRequirement,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const DECISION_PROBABILITY_TOLERANCE: f64 = 1e-4;
pub const DECISION_MAX_INPUT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DecisionQuestionType {
    Choice,
    Score,
    Boolean,
}

impl DecisionQuestionType {
    pub fn feature(self) -> &'static str {
        match self {
            Self::Choice => "decision.choice",
            Self::Score => "decision.score",
            Self::Boolean => "decision.boolean",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DecisionOption {
    pub id: String,
    pub description: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DecisionBooleanCriteria {
    #[serde(rename = "true", default, skip_serializing_if = "Option::is_none")]
    pub when_true: Option<Value>,
    #[serde(rename = "false", default, skip_serializing_if = "Option::is_none")]
    pub when_false: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DecisionQuestion {
    Choice {
        id: String,
        instructions: Value,
        options: Vec<DecisionOption>,
    },
    Score {
        id: String,
        instructions: Value,
        levels: Vec<Value>,
    },
    Boolean {
        id: String,
        instructions: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<DecisionBooleanCriteria>,
    },
}

impl DecisionQuestion {
    pub fn id(&self) -> &str {
        match self {
            Self::Choice { id, .. } | Self::Score { id, .. } | Self::Boolean { id, .. } => id,
        }
    }

    pub fn question_type(&self) -> DecisionQuestionType {
        match self {
            Self::Choice { .. } => DecisionQuestionType::Choice,
            Self::Score { .. } => DecisionQuestionType::Score,
            Self::Boolean { .. } => DecisionQuestionType::Boolean,
        }
    }

    pub fn instructions(&self) -> &Value {
        match self {
            Self::Choice { instructions, .. }
            | Self::Score { instructions, .. }
            | Self::Boolean { instructions, .. } => instructions,
        }
    }

    fn rules(&self) -> Vec<&Value> {
        let mut values = vec![self.instructions()];
        match self {
            Self::Choice { options, .. } => values.extend(options.iter().map(|v| &v.description)),
            Self::Score { levels, .. } => values.extend(levels),
            Self::Boolean {
                criteria: Some(criteria),
                ..
            } => {
                values.extend(criteria.when_true.iter());
                values.extend(criteria.when_false.iter());
            }
            _ => {}
        }
        values
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DecisionAnswer {
    Choice {
        id: String,
        selected: String,
        probabilities: BTreeMap<String, f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<f64>,
    },
    Score {
        id: String,
        score: f64,
        levels: Vec<Value>,
        probabilities: BTreeMap<String, f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<f64>,
    },
    Boolean {
        id: String,
        probability_true: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<f64>,
    },
}

impl DecisionAnswer {
    pub fn id(&self) -> &str {
        match self {
            Self::Choice { id, .. } | Self::Score { id, .. } | Self::Boolean { id, .. } => id,
        }
    }

    pub fn confidence(&self) -> Option<f64> {
        match self {
            Self::Choice { confidence, .. }
            | Self::Score { confidence, .. }
            | Self::Boolean { confidence, .. } => *confidence,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DecisionRequirements {
    #[serde(default)]
    pub question_types: BTreeSet<DecisionQuestionType>,
    #[serde(default)]
    pub structured_state: bool,
    #[serde(default)]
    pub structured_rules: bool,
    #[serde(default)]
    pub question_count: u64,
    #[serde(default)]
    pub max_options: u64,
    #[serde(default)]
    pub max_levels: u64,
    #[serde(default)]
    pub input_bytes: u64,
    #[serde(default)]
    pub max_state_question_bytes: u64,
}

impl DecisionRequirements {
    pub fn merge(&mut self, other: &Self) {
        self.question_types.extend(&other.question_types);
        self.structured_state |= other.structured_state;
        self.structured_rules |= other.structured_rules;
        self.question_count = self.question_count.max(other.question_count);
        self.max_options = self.max_options.max(other.max_options);
        self.max_levels = self.max_levels.max(other.max_levels);
        self.input_bytes = self.input_bytes.max(other.input_bytes);
        self.max_state_question_bytes = self
            .max_state_question_bytes
            .max(other.max_state_question_bytes);
    }

    pub fn features(&self) -> Vec<String> {
        let mut features = vec!["decision.probabilities".to_owned()];
        features.extend(self.question_types.iter().map(|ty| ty.feature().to_owned()));
        if self.structured_state {
            features.push("decision.structured_state".to_owned());
        }
        if self.structured_rules {
            features.push("decision.structured_rules".to_owned());
        }
        features
    }

    pub fn limits(&self) -> [(&'static str, u64); 5] {
        [
            ("decision.max_questions", self.question_count),
            ("decision.max_options", self.max_options),
            ("decision.max_levels", self.max_levels),
            ("decision.max_input_bytes", self.input_bytes),
            (
                "decision.max_state_question_bytes",
                self.max_state_question_bytes,
            ),
        ]
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
}

fn structured_text(value: &Value) -> bool {
    match value {
        Value::String(v) => !v.trim().is_empty(),
        Value::Object(v) => !v.is_empty(),
        Value::Array(v) => !v.is_empty(),
        _ => false,
    }
}

fn invalid(message: &str) -> AiccError {
    AiccError::new(AiccErrorCode::InvalidRequest, message)
}

fn invalid_answer(message: &str) -> AiccError {
    AiccError::new(AiccErrorCode::ProviderError, message)
}

impl DecisionEvaluateRequest {
    pub fn validate(&self) -> Result<(), AiccError> {
        validate_exact_model_name(&self.exact_model)?;
        if !matches!(
            self.state,
            Value::String(_) | Value::Object(_) | Value::Array(_)
        ) {
            return Err(invalid("decision state must be a string, object or array"));
        }
        if self.questions.is_empty() || self.questions.len() > 1024 {
            return Err(invalid("decision requires 1..1024 questions"));
        }
        let mut ids = BTreeSet::new();
        for question in &self.questions {
            if !valid_id(question.id()) || !ids.insert(question.id()) {
                return Err(invalid("decision question IDs must be valid and unique"));
            }
            if !structured_text(question.instructions()) {
                return Err(invalid(
                    "decision instructions must be nonempty text, object or array",
                ));
            }
            match question {
                DecisionQuestion::Choice { options, .. } => {
                    if options.is_empty() || options.len() > 1024 {
                        return Err(invalid("decision choice requires 1..1024 options"));
                    }
                    let mut ids = BTreeSet::new();
                    for option in options {
                        if !valid_id(&option.id)
                            || !ids.insert(&option.id)
                            || !(option.description.is_null()
                                || structured_text(&option.description))
                        {
                            return Err(invalid("decision choice options must have unique valid IDs and descriptions"));
                        }
                    }
                }
                DecisionQuestion::Score { levels, .. } => {
                    if !(2..=1024).contains(&levels.len())
                        || levels.iter().any(|v| !structured_text(v))
                        || levels
                            .iter()
                            .enumerate()
                            .any(|(i, v)| levels[..i].contains(v))
                    {
                        return Err(invalid(
                            "decision score requires 2..1024 distinct nonempty levels",
                        ));
                    }
                }
                DecisionQuestion::Boolean {
                    criteria: Some(criteria),
                    ..
                } => {
                    if criteria
                        .when_true
                        .iter()
                        .chain(criteria.when_false.iter())
                        .any(|v| !structured_text(v))
                    {
                        return Err(invalid(
                            "decision boolean criteria must contain text, objects or arrays",
                        ));
                    }
                }
                _ => {}
            }
        }
        if self.decision_requirements().input_bytes > DECISION_MAX_INPUT_BYTES as u64 {
            return Err(invalid("decision input exceeds 1 MiB"));
        }
        Ok(())
    }

    pub fn decision_requirements(&self) -> DecisionRequirements {
        let state_bytes = serde_json::to_vec(&self.state)
            .expect("JSON state serializes")
            .len();
        let questions_bytes = serde_json::to_vec(&self.questions)
            .expect("decision questions serialize")
            .len();
        let mut result = DecisionRequirements {
            structured_state: !self.state.is_string(),
            question_count: self.questions.len() as u64,
            input_bytes: (state_bytes + questions_bytes) as u64,
            ..Default::default()
        };
        for question in &self.questions {
            result.question_types.insert(question.question_type());
            result.structured_rules |= question
                .rules()
                .iter()
                .any(|v| v.is_object() || v.is_array());
            result.max_state_question_bytes = result.max_state_question_bytes.max(
                (state_bytes
                    + serde_json::to_vec(question)
                        .expect("question serializes")
                        .len()) as u64,
            );
            match question {
                DecisionQuestion::Choice { options, .. } => {
                    result.max_options = result.max_options.max(options.len() as u64)
                }
                DecisionQuestion::Score { levels, .. } => {
                    result.max_levels = result.max_levels.max(levels.len() as u64)
                }
                _ => {}
            }
        }
        result
    }

    pub fn requirements(&self) -> ModelRequirement {
        ModelRequirement {
            decision: Some(self.decision_requirements()),
            streaming: self.execution_mode == crate::AiccExecutionMode::Stream,
            ..Default::default()
        }
    }

    pub fn validate_answers(&self, answers: &[DecisionAnswer]) -> Result<(), AiccError> {
        if answers.len() != self.questions.len() {
            return Err(invalid_answer(
                "decision answers must cover every question exactly once",
            ));
        }
        let mut ids = BTreeSet::new();
        for answer in answers {
            if !ids.insert(answer.id()) || answer.confidence().is_some_and(|v| !probability(v)) {
                return Err(invalid_answer(
                    "decision answer has a duplicate ID or invalid confidence",
                ));
            }
            let question = self
                .questions
                .iter()
                .find(|q| q.id() == answer.id())
                .ok_or_else(|| invalid_answer("decision answer has an unknown question ID"))?;
            match (question, answer) {
                (
                    DecisionQuestion::Choice { options, .. },
                    DecisionAnswer::Choice {
                        selected,
                        probabilities,
                        ..
                    },
                ) => {
                    distribution(probabilities, options.iter().map(|o| o.id.clone()))?;
                    let selected_probability = probabilities.get(selected).ok_or_else(|| {
                        invalid_answer("decision selected option is outside the candidate set")
                    })?;
                    if probabilities
                        .values()
                        .any(|p| p > &(selected_probability + DECISION_PROBABILITY_TOLERANCE))
                    {
                        return Err(invalid_answer(
                            "decision selected option is not a maximum probability option",
                        ));
                    }
                }
                (
                    DecisionQuestion::Score { levels, .. },
                    DecisionAnswer::Score {
                        score,
                        levels: actual,
                        probabilities,
                        ..
                    },
                ) => {
                    distribution(probabilities, (0..levels.len()).map(|i| i.to_string()))?;
                    let expected: f64 = (0..levels.len())
                        .map(|i| i as f64 * probabilities[&i.to_string()])
                        .sum();
                    if actual != levels
                        || !score.is_finite()
                        || *score < 0.0
                        || *score > (levels.len() - 1) as f64
                        || (score - expected).abs()
                            > DECISION_PROBABILITY_TOLERANCE * (levels.len() - 1) as f64
                    {
                        return Err(invalid_answer(
                            "decision score or level correspondence is invalid",
                        ));
                    }
                }
                (
                    DecisionQuestion::Boolean { .. },
                    DecisionAnswer::Boolean {
                        probability_true, ..
                    },
                ) => {
                    if !probability(*probability_true) {
                        return Err(invalid_answer("decision boolean probability is invalid"));
                    }
                }
                _ => {
                    return Err(invalid_answer(
                        "decision answer type does not match its question",
                    ))
                }
            }
        }
        Ok(())
    }
}

fn probability(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn distribution(
    values: &BTreeMap<String, f64>,
    ids: impl Iterator<Item = String>,
) -> Result<(), AiccError> {
    let keys: BTreeSet<_> = ids.collect();
    if values.keys().cloned().collect::<BTreeSet<_>>() != keys
        || values.values().any(|v| !probability(*v))
        || (values.values().sum::<f64>() - 1.0).abs() > DECISION_PROBABILITY_TOLERANCE
    {
        return Err(invalid_answer(
            "decision probability distribution is invalid or incomplete",
        ));
    }
    Ok(())
}
