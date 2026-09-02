#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum MatchRule {
    Shorthand(String),
    Object(BTreeMap<String, Value>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DimensionType {
    String,
    Number,
    Version,
    Boolean,
    JsonScalar,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DimensionSpec {
    pub name: &'static str,
    pub value_type: DimensionType,
    pub allow_range: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MatchSchema {
    pub name: &'static str,
    pub primary_dimension: Option<&'static str>,
    pub dimensions: &'static [DimensionSpec],
    pub allow_json_pointer_dimensions: bool,
}

const MODEL_DRIVER_DIMENSIONS: &[DimensionSpec] = &[
    string_dimension("origin_model_id"),
    string_dimension("family"),
    string_dimension("tier"),
    string_dimension("stability"),
    string_dimension("api_type"),
];

const PROVIDER_RULE_DIMENSIONS: &[DimensionSpec] = &[
    string_dimension("provider_model_id"),
    string_dimension("origin_model_id"),
    string_dimension("model_driver_id"),
    string_dimension("variant"),
    string_dimension("api_type"),
];

const REQUEST_DIMENSIONS: &[DimensionSpec] =
    &[string_dimension("api_type"), string_dimension("operation")];

const ROUTING_PROVIDER_DIMENSIONS: &[DimensionSpec] = &[
    string_dimension("provider_instance_name"),
    string_dimension("api_type"),
    string_dimension("logical_path"),
];

const ROUTING_MODEL_DIMENSIONS: &[DimensionSpec] = &[
    string_dimension("exact_model"),
    string_dimension("api_type"),
    string_dimension("logical_path"),
];

const RELEASE_TRACK_DIMENSIONS: &[DimensionSpec] = &[
    DimensionSpec {
        name: "client_version",
        value_type: DimensionType::Version,
        allow_range: true,
    },
    string_dimension("update_channel"),
    string_dimension("rollout_group"),
];

const fn string_dimension(name: &'static str) -> DimensionSpec {
    DimensionSpec {
        name,
        value_type: DimensionType::String,
        allow_range: false,
    }
}

pub(crate) const MODEL_DRIVER_MATCH_SCHEMA: MatchSchema = MatchSchema {
    name: "model_driver",
    primary_dimension: Some("origin_model_id"),
    dimensions: MODEL_DRIVER_DIMENSIONS,
    allow_json_pointer_dimensions: false,
};

pub(crate) const PROVIDER_RULE_MATCH_SCHEMA: MatchSchema = MatchSchema {
    name: "provider_rule",
    primary_dimension: Some("provider_model_id"),
    dimensions: PROVIDER_RULE_DIMENSIONS,
    allow_json_pointer_dimensions: false,
};

pub(crate) const REQUEST_RULE_MATCH_SCHEMA: MatchSchema = MatchSchema {
    name: "request_rule",
    primary_dimension: None,
    dimensions: REQUEST_DIMENSIONS,
    allow_json_pointer_dimensions: true,
};

pub(crate) const PRICING_RULE_MATCH_SCHEMA: MatchSchema = MatchSchema {
    name: "pricing_rule",
    primary_dimension: None,
    dimensions: REQUEST_DIMENSIONS,
    allow_json_pointer_dimensions: true,
};

pub(crate) const ROUTING_PROVIDER_MATCH_SCHEMA: MatchSchema = MatchSchema {
    name: "routing_provider_scope",
    primary_dimension: Some("provider_instance_name"),
    dimensions: ROUTING_PROVIDER_DIMENSIONS,
    allow_json_pointer_dimensions: false,
};

pub(crate) const ROUTING_MODEL_MATCH_SCHEMA: MatchSchema = MatchSchema {
    name: "routing_model_scope",
    primary_dimension: Some("exact_model"),
    dimensions: ROUTING_MODEL_DIMENSIONS,
    allow_json_pointer_dimensions: false,
};

pub(crate) const RELEASE_TRACK_MATCH_SCHEMA: MatchSchema = MatchSchema {
    name: "release_track",
    primary_dimension: Some("client_version"),
    dimensions: RELEASE_TRACK_DIMENSIONS,
    allow_json_pointer_dimensions: false,
};

pub(crate) type MatchContext = BTreeMap<String, Value>;

#[derive(Clone, Debug)]
pub(crate) struct CompiledMatchRule {
    dimensions: Vec<CompiledDimension>,
}

impl CompiledMatchRule {
    pub(crate) fn compile(
        rule: MatchRule,
        schema: &MatchSchema,
    ) -> Result<Self, MatchCompileError> {
        let fields = match rule {
            MatchRule::Shorthand(pattern) => {
                let primary = schema.primary_dimension.ok_or_else(|| {
                    MatchCompileError::new(schema, None, MatchCompileErrorKind::ShorthandNotAllowed)
                })?;
                BTreeMap::from([(primary.to_owned(), Value::String(pattern))])
            }
            MatchRule::Object(fields) => fields,
        };

        if fields.is_empty() {
            return Err(MatchCompileError::new(
                schema,
                None,
                MatchCompileErrorKind::EmptyObject,
            ));
        }

        let mut dimensions = Vec::with_capacity(fields.len());
        for (name, condition) in fields {
            let value_type = schema.dimension_type(&name).ok_or_else(|| {
                MatchCompileError::new(
                    schema,
                    Some(name.clone()),
                    MatchCompileErrorKind::UnknownDimension,
                )
            })?;
            let allow_range = schema.dimension_allows_range(&name, value_type);
            let predicate = compile_condition(&condition, value_type, allow_range)
                .map_err(|kind| MatchCompileError::new(schema, Some(name.clone()), kind))?;
            dimensions.push(CompiledDimension { name, predicate });
        }

        Ok(Self { dimensions })
    }

    pub(crate) fn matches(&self, context: &MatchContext) -> bool {
        self.dimensions
            .iter()
            .all(|dimension| dimension.matches(context.get(&dimension.name)))
    }

    pub(crate) fn participating_dimensions(&self) -> impl Iterator<Item = &str> {
        self.dimensions
            .iter()
            .map(|dimension| dimension.name.as_str())
    }
}

impl MatchSchema {
    fn dimension_type(&self, name: &str) -> Option<DimensionType> {
        self.dimensions
            .iter()
            .find(|dimension| dimension.name == name)
            .map(|dimension| dimension.value_type)
            .or_else(|| {
                (self.allow_json_pointer_dimensions && is_normalized_json_pointer(name))
                    .then_some(DimensionType::JsonScalar)
            })
    }

    fn dimension_allows_range(&self, name: &str, value_type: DimensionType) -> bool {
        self.dimensions
            .iter()
            .find(|dimension| dimension.name == name)
            .map(|dimension| dimension.allow_range)
            .unwrap_or(value_type == DimensionType::JsonScalar)
    }
}

fn is_normalized_json_pointer(value: &str) -> bool {
    value.starts_with('/')
        && value.bytes().enumerate().all(|(index, byte)| {
            byte != b'~' || matches!(value.as_bytes().get(index + 1), Some(b'0' | b'1'))
        })
}

#[derive(Clone, Debug)]
struct CompiledDimension {
    name: String,
    predicate: CompiledPredicate,
}

impl CompiledDimension {
    fn matches(&self, actual: Option<&Value>) -> bool {
        self.predicate.matches(actual)
    }
}

#[derive(Clone, Debug)]
enum CompiledPredicate {
    Scalar(CompiledScalar),
    Any(Vec<CompiledScalar>),
    Not(Box<CompiledPredicate>),
    Exists(bool),
    Range(CompiledRange),
}

impl CompiledPredicate {
    fn matches(&self, actual: Option<&Value>) -> bool {
        match self {
            Self::Scalar(expected) => actual.is_some_and(|actual| expected.matches(actual)),
            Self::Any(expected) => actual
                .is_some_and(|actual| expected.iter().any(|expected| expected.matches(actual))),
            Self::Not(inner) => !inner.matches(actual),
            Self::Exists(expected) => actual.is_some() == *expected,
            Self::Range(range) => actual.is_some_and(|actual| range.matches(actual)),
        }
    }
}

#[derive(Clone, Debug)]
enum CompiledScalar {
    String(CompiledGlob),
    Number(serde_json::Number),
    Boolean(bool),
    Null,
}

impl CompiledScalar {
    fn matches(&self, actual: &Value) -> bool {
        match (self, actual) {
            (Self::String(expected), Value::String(actual)) => expected.matches(actual),
            (Self::Number(expected), Value::Number(actual)) => expected == actual,
            (Self::Boolean(expected), Value::Bool(actual)) => expected == actual,
            (Self::Null, Value::Null) => true,
            _ => false,
        }
    }
}

fn compile_condition(
    value: &Value,
    value_type: DimensionType,
    allow_range: bool,
) -> Result<CompiledPredicate, MatchCompileErrorKind> {
    match value {
        Value::Array(values) => {
            let compiled = values
                .iter()
                .map(|value| compile_scalar(value, value_type))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(CompiledPredicate::Any(compiled))
        }
        Value::Object(operator) => compile_operator(operator, value_type, allow_range),
        _ => Ok(CompiledPredicate::Scalar(compile_scalar(
            value, value_type,
        )?)),
    }
}

fn compile_operator(
    operator: &serde_json::Map<String, Value>,
    value_type: DimensionType,
    allow_range: bool,
) -> Result<CompiledPredicate, MatchCompileErrorKind> {
    if operator.len() == 1 {
        if let Some(value) = operator.get("not") {
            let inner = match value {
                Value::Array(values) => CompiledPredicate::Any(
                    values
                        .iter()
                        .map(|value| compile_scalar(value, value_type))
                        .collect::<Result<Vec<_>, _>>()?,
                ),
                Value::Object(_) => return Err(MatchCompileErrorKind::InvalidNotOperand),
                _ => CompiledPredicate::Scalar(compile_scalar(value, value_type)?),
            };
            return Ok(CompiledPredicate::Not(Box::new(inner)));
        }
        if let Some(value) = operator.get("exists") {
            return value
                .as_bool()
                .map(CompiledPredicate::Exists)
                .ok_or(MatchCompileErrorKind::InvalidExistsOperand);
        }
    }

    const RANGE_KEYS: &[&str] = &["min", "max", "include_min", "include_max"];
    if operator
        .keys()
        .all(|key| RANGE_KEYS.contains(&key.as_str()))
    {
        return compile_range(operator, value_type, allow_range).map(CompiledPredicate::Range);
    }

    Err(MatchCompileErrorKind::InvalidOperator)
}

fn compile_scalar(
    value: &Value,
    value_type: DimensionType,
) -> Result<CompiledScalar, MatchCompileErrorKind> {
    match (value_type, value) {
        (DimensionType::String | DimensionType::Version, Value::String(value)) => {
            Ok(CompiledScalar::String(CompiledGlob::compile(value)?))
        }
        (DimensionType::Number, Value::Number(value)) => Ok(CompiledScalar::Number(value.clone())),
        (DimensionType::Boolean, Value::Bool(value)) => Ok(CompiledScalar::Boolean(*value)),
        (DimensionType::JsonScalar, Value::String(value)) => {
            Ok(CompiledScalar::String(CompiledGlob::compile(value)?))
        }
        (DimensionType::JsonScalar, Value::Number(value)) => {
            Ok(CompiledScalar::Number(value.clone()))
        }
        (DimensionType::JsonScalar, Value::Bool(value)) => Ok(CompiledScalar::Boolean(*value)),
        (DimensionType::JsonScalar, Value::Null) => Ok(CompiledScalar::Null),
        _ => Err(MatchCompileErrorKind::InvalidValueType),
    }
}

#[derive(Clone, Debug)]
struct CompiledRange {
    min: Option<RangeBound>,
    max: Option<RangeBound>,
    include_min: bool,
    include_max: bool,
}

#[derive(Clone, Debug)]
enum RangeBound {
    Number(serde_json::Number),
    Version(ParsedVersion),
}

impl RangeBound {
    fn compare_actual(&self, actual: &Value) -> Option<Ordering> {
        match self {
            Self::Number(expected) => actual
                .as_number()
                .and_then(|actual| compare_numbers(actual, expected)),
            Self::Version(expected) => actual
                .as_str()
                .and_then(|actual| ParsedVersion::parse(actual).ok())
                .map(|actual| actual.cmp(expected)),
        }
    }
}

impl CompiledRange {
    fn matches(&self, actual: &Value) -> bool {
        if let Some(min) = &self.min {
            let Some(ordering) = min.compare_actual(actual) else {
                return false;
            };
            if ordering == Ordering::Less || (!self.include_min && ordering == Ordering::Equal) {
                return false;
            }
        }
        if let Some(max) = &self.max {
            let Some(ordering) = max.compare_actual(actual) else {
                return false;
            };
            if ordering == Ordering::Greater || (!self.include_max && ordering == Ordering::Equal) {
                return false;
            }
        }
        true
    }
}

fn compile_range(
    operator: &serde_json::Map<String, Value>,
    value_type: DimensionType,
    allow_range: bool,
) -> Result<CompiledRange, MatchCompileErrorKind> {
    if !allow_range {
        return Err(MatchCompileErrorKind::RangeNotAllowed);
    }
    if operator.get("min").is_none() && operator.get("max").is_none() {
        return Err(MatchCompileErrorKind::EmptyRange);
    }
    if operator.contains_key("include_min") && !operator.contains_key("min")
        || operator.contains_key("include_max") && !operator.contains_key("max")
    {
        return Err(MatchCompileErrorKind::RangeFlagWithoutBound);
    }

    let range_type = match value_type {
        DimensionType::Version => RangeType::Version,
        DimensionType::Number | DimensionType::JsonScalar => RangeType::Number,
        _ => return Err(MatchCompileErrorKind::RangeNotAllowed),
    };
    let min = operator
        .get("min")
        .map(|value| compile_range_bound(value, range_type))
        .transpose()?;
    let max = operator
        .get("max")
        .map(|value| compile_range_bound(value, range_type))
        .transpose()?;
    let include_min = read_range_flag(operator, "include_min")?.unwrap_or(true);
    let include_max = read_range_flag(operator, "include_max")?.unwrap_or(true);

    if let (Some(min), Some(max)) = (&min, &max) {
        let ordering = match (min, max) {
            (RangeBound::Number(min), RangeBound::Number(max)) => compare_numbers(min, max),
            (RangeBound::Version(min), RangeBound::Version(max)) => Some(min.cmp(max)),
            _ => None,
        };
        if !matches!(ordering, Some(Ordering::Less | Ordering::Equal)) {
            return Err(MatchCompileErrorKind::ReversedRange);
        }
    }

    Ok(CompiledRange {
        min,
        max,
        include_min,
        include_max,
    })
}

#[derive(Clone, Copy)]
enum RangeType {
    Number,
    Version,
}

fn compile_range_bound(
    value: &Value,
    range_type: RangeType,
) -> Result<RangeBound, MatchCompileErrorKind> {
    match (range_type, value) {
        (RangeType::Number, Value::Number(value)) => value
            .as_f64()
            .map(|_| RangeBound::Number(value.clone()))
            .ok_or(MatchCompileErrorKind::InvalidRangeBound),
        (RangeType::Version, Value::String(value)) => ParsedVersion::parse(value)
            .map(RangeBound::Version)
            .map_err(|_| MatchCompileErrorKind::InvalidRangeBound),
        _ => Err(MatchCompileErrorKind::InvalidRangeBound),
    }
}

fn compare_numbers(left: &serde_json::Number, right: &serde_json::Number) -> Option<Ordering> {
    match (left.as_i64(), right.as_i64()) {
        (Some(left), Some(right)) => return Some(left.cmp(&right)),
        (Some(left), None) if right.as_u64().is_some() => {
            return Some(if left < 0 {
                Ordering::Less
            } else {
                (left as u64).cmp(&right.as_u64().expect("checked above"))
            });
        }
        (None, Some(right)) if left.as_u64().is_some() => {
            return Some(if right < 0 {
                Ordering::Greater
            } else {
                left.as_u64().expect("checked above").cmp(&(right as u64))
            });
        }
        _ => {}
    }
    match (left.as_u64(), right.as_u64()) {
        (Some(left), Some(right)) => Some(left.cmp(&right)),
        _ => left
            .as_f64()
            .and_then(|left| right.as_f64().and_then(|right| left.partial_cmp(&right))),
    }
}

fn read_range_flag(
    operator: &serde_json::Map<String, Value>,
    name: &str,
) -> Result<Option<bool>, MatchCompileErrorKind> {
    operator
        .get(name)
        .map(|value| {
            value
                .as_bool()
                .ok_or(MatchCompileErrorKind::InvalidRangeFlag)
        })
        .transpose()
}

#[derive(Clone, Debug)]
struct CompiledGlob {
    tokens: Vec<GlobToken>,
}

#[derive(Clone, Debug)]
enum GlobToken {
    Literal(char),
    AnySequence,
    AnyChar,
}

impl CompiledGlob {
    fn compile(pattern: &str) -> Result<Self, MatchCompileErrorKind> {
        let mut chars = pattern.chars();
        let mut tokens = Vec::with_capacity(pattern.len());
        while let Some(character) = chars.next() {
            match character {
                '*' => {
                    if !matches!(tokens.last(), Some(GlobToken::AnySequence)) {
                        tokens.push(GlobToken::AnySequence);
                    }
                }
                '?' => tokens.push(GlobToken::AnyChar),
                '\\' => match chars.next() {
                    Some(escaped @ ('*' | '?' | '\\')) => tokens.push(GlobToken::Literal(escaped)),
                    _ => return Err(MatchCompileErrorKind::InvalidEscape),
                },
                literal => tokens.push(GlobToken::Literal(literal)),
            }
        }
        Ok(Self { tokens })
    }

    fn matches(&self, value: &str) -> bool {
        let value: Vec<char> = value.chars().collect();
        let (mut token_index, mut value_index) = (0, 0);
        let (mut star_token, mut star_value) = (None, 0);

        while value_index < value.len() {
            match self.tokens.get(token_index) {
                Some(GlobToken::Literal(expected)) if *expected == value[value_index] => {
                    token_index += 1;
                    value_index += 1;
                }
                Some(GlobToken::AnyChar) => {
                    token_index += 1;
                    value_index += 1;
                }
                Some(GlobToken::AnySequence) => {
                    star_token = Some(token_index);
                    token_index += 1;
                    star_value = value_index;
                }
                _ if star_token.is_some() => {
                    star_value += 1;
                    value_index = star_value;
                    token_index = star_token.expect("checked above") + 1;
                }
                _ => return false,
            }
        }

        while matches!(self.tokens.get(token_index), Some(GlobToken::AnySequence)) {
            token_index += 1;
        }
        token_index == self.tokens.len()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParsedVersion {
    core: Vec<u64>,
    prerelease: Option<Vec<VersionIdentifier>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum VersionIdentifier {
    Numeric(u64),
    Text(String),
}

impl ParsedVersion {
    fn parse(input: &str) -> Result<Self, ()> {
        let (without_build, build) = input
            .split_once('+')
            .map_or((input, None), |(version, build)| (version, Some(build)));
        if input.is_empty()
            || without_build.contains('+')
            || build.is_some_and(|build| !valid_version_identifiers(build))
        {
            return Err(());
        }
        let (core, prerelease) = without_build
            .split_once('-')
            .map_or((without_build, None), |(core, prerelease)| {
                (core, Some(prerelease))
            });
        let core = core
            .split('.')
            .map(|part| {
                (!part.is_empty())
                    .then(|| part.parse::<u64>().ok())
                    .flatten()
                    .ok_or(())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if core.is_empty() {
            return Err(());
        }
        let prerelease = prerelease
            .map(|value| {
                if !valid_version_identifiers(value) {
                    return Err(());
                }
                value
                    .split('.')
                    .map(|part| {
                        Ok(part.parse::<u64>().map_or_else(
                            |_| VersionIdentifier::Text(part.to_owned()),
                            VersionIdentifier::Numeric,
                        ))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
        Ok(Self { core, prerelease })
    }
}

fn valid_version_identifiers(value: &str) -> bool {
    !value.is_empty()
        && value.split('.').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
}

impl Ord for ParsedVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        let core_length = self.core.len().max(other.core.len());
        for index in 0..core_length {
            let ordering = self
                .core
                .get(index)
                .copied()
                .unwrap_or(0)
                .cmp(&other.core.get(index).copied().unwrap_or(0));
            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        match (&self.prerelease, &other.prerelease) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(left), Some(right)) => compare_prerelease(left, right),
        }
    }
}

impl PartialOrd for ParsedVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn compare_prerelease(left: &[VersionIdentifier], right: &[VersionIdentifier]) -> Ordering {
    for (left, right) in left.iter().zip(right) {
        let ordering = match (left, right) {
            (VersionIdentifier::Numeric(left), VersionIdentifier::Numeric(right)) => {
                left.cmp(right)
            }
            (VersionIdentifier::Numeric(_), VersionIdentifier::Text(_)) => Ordering::Less,
            (VersionIdentifier::Text(_), VersionIdentifier::Numeric(_)) => Ordering::Greater,
            (VersionIdentifier::Text(left), VersionIdentifier::Text(right)) => left.cmp(right),
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.len().cmp(&right.len())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MatchCompileError {
    pub schema: &'static str,
    pub dimension: Option<String>,
    pub kind: MatchCompileErrorKind,
}

impl MatchCompileError {
    fn new(schema: &MatchSchema, dimension: Option<String>, kind: MatchCompileErrorKind) -> Self {
        Self {
            schema: schema.name,
            dimension,
            kind,
        }
    }
}

impl fmt::Display for MatchCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid {} match rule", self.schema)?;
        if let Some(dimension) = &self.dimension {
            write!(formatter, " dimension {dimension:?}")?;
        }
        write!(formatter, ": {}", self.kind)
    }
}

impl Error for MatchCompileError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MatchCompileErrorKind {
    ShorthandNotAllowed,
    EmptyObject,
    UnknownDimension,
    InvalidValueType,
    InvalidOperator,
    InvalidNotOperand,
    InvalidExistsOperand,
    RangeNotAllowed,
    EmptyRange,
    RangeFlagWithoutBound,
    InvalidRangeFlag,
    InvalidRangeBound,
    ReversedRange,
    InvalidEscape,
}

impl fmt::Display for MatchCompileErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ShorthandNotAllowed => "string shorthand is not allowed",
            Self::EmptyObject => "an object rule must contain at least one dimension",
            Self::UnknownDimension => "unknown dimension",
            Self::InvalidValueType => "value does not match the dimension type",
            Self::InvalidOperator => "unknown or incompatible operator",
            Self::InvalidNotOperand => "not accepts only a scalar or scalar array",
            Self::InvalidExistsOperand => "exists requires a boolean",
            Self::RangeNotAllowed => "range is not allowed for this dimension",
            Self::EmptyRange => "range requires min or max",
            Self::RangeFlagWithoutBound => "inclusive flag requires its corresponding bound",
            Self::InvalidRangeFlag => "range inclusive flag requires a boolean",
            Self::InvalidRangeBound => "range bound does not match the ordered dimension type",
            Self::ReversedRange => "range min is greater than max",
            Self::InvalidEscape => "only *, ? and backslash may be escaped",
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RuleEntry {
    pub rule_id: Option<String>,
    pub rule: MatchRule,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledRuleSet {
    rules: Vec<CompiledRuleEntry>,
}

#[derive(Clone, Debug)]
struct CompiledRuleEntry {
    rule_id: Option<String>,
    position: usize,
    rule: CompiledMatchRule,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MatchTrace {
    pub rule_id: Option<String>,
    pub position: usize,
    pub dimensions: Vec<String>,
}

impl CompiledRuleSet {
    pub(crate) fn compile(
        entries: impl IntoIterator<Item = RuleEntry>,
        schema: &MatchSchema,
    ) -> Result<Self, MatchCompileError> {
        let rules = entries
            .into_iter()
            .enumerate()
            .map(|(position, entry)| {
                Ok(CompiledRuleEntry {
                    rule_id: entry.rule_id,
                    position,
                    rule: CompiledMatchRule::compile(entry.rule, schema)?,
                })
            })
            .collect::<Result<Vec<_>, MatchCompileError>>()?;
        Ok(Self { rules })
    }

    pub(crate) fn first_match(&self, context: &MatchContext) -> Option<MatchTrace> {
        self.rules.iter().find_map(|entry| {
            entry.rule.matches(context).then(|| MatchTrace {
                rule_id: entry.rule_id.clone(),
                position: entry.position,
                dimensions: entry
                    .rule
                    .participating_dimensions()
                    .map(str::to_owned)
                    .collect(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn compile(json: Value, schema: &MatchSchema) -> Result<CompiledMatchRule, MatchCompileError> {
        CompiledMatchRule::compile(serde_json::from_value(json).unwrap(), schema)
    }

    fn context(values: &[(&str, Value)]) -> MatchContext {
        values
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect()
    }

    #[test]
    fn exact_wildcard_question_and_escape_are_full_string_matches() {
        let cases = [
            (json!("gpt-5"), "gpt-5", true),
            (json!("gpt-5"), "prefix-gpt-5", false),
            (json!("gpt-*-mini"), "gpt-5-mini", true),
            (json!("gpt-?-mini"), "gpt-五-mini", true),
            (json!(r"literal\*\?\\"), r"literal*?\", true),
            (json!("*"), "", true),
        ];
        for (rule, actual, expected) in cases {
            let compiled = compile(rule, &MODEL_DRIVER_MATCH_SCHEMA).unwrap();
            assert_eq!(
                compiled.matches(&context(&[("origin_model_id", json!(actual))])),
                expected
            );
        }
        assert_eq!(
            compile(json!(r"bad\q"), &MODEL_DRIVER_MATCH_SCHEMA)
                .unwrap_err()
                .kind,
            MatchCompileErrorKind::InvalidEscape
        );
    }

    #[test]
    fn shorthand_and_single_dimension_object_are_equivalent() {
        let shorthand = compile(json!("gpt-5-*"), &MODEL_DRIVER_MATCH_SCHEMA).unwrap();
        let object = compile(
            json!({"origin_model_id": "gpt-5-*"}),
            &MODEL_DRIVER_MATCH_SCHEMA,
        )
        .unwrap();
        for actual in ["gpt-5-mini", "claude-4"] {
            let context = context(&[("origin_model_id", json!(actual))]);
            assert_eq!(shorthand.matches(&context), object.matches(&context));
        }
    }

    #[test]
    fn dimensions_are_and_and_array_values_are_or() {
        let compiled = compile(
            json!({
                "provider_model_id": "openai/gpt-5*",
                "api_type": ["llm", "vision.*"]
            }),
            &PROVIDER_RULE_MATCH_SCHEMA,
        )
        .unwrap();
        assert!(compiled.matches(&context(&[
            ("provider_model_id", json!("openai/gpt-5-mini")),
            ("api_type", json!("llm")),
        ])));
        assert!(!compiled.matches(&context(&[
            ("provider_model_id", json!("openai/gpt-5-mini")),
            ("api_type", json!("image.txt2img")),
        ])));
        let never = compile(
            json!({"provider_model_id": []}),
            &PROVIDER_RULE_MATCH_SCHEMA,
        )
        .unwrap();
        assert!(!never.matches(&context(&[("provider_model_id", json!("anything"))])));
    }

    #[test]
    fn not_and_exists_have_deterministic_missing_value_behavior() {
        let not = compile(
            json!({"api_type": {"not": ["image.*", "video.*"]}}),
            &PROVIDER_RULE_MATCH_SCHEMA,
        )
        .unwrap();
        assert!(not.matches(&context(&[("api_type", json!("llm"))])));
        assert!(not.matches(&MatchContext::new()));
        assert!(!not.matches(&context(&[("api_type", json!("image.txt2img"))])));

        let exists = compile(
            json!({"origin_model_id": {"exists": false}}),
            &PROVIDER_RULE_MATCH_SCHEMA,
        )
        .unwrap();
        assert!(exists.matches(&MatchContext::new()));
        assert!(!exists.matches(&context(&[("origin_model_id", Value::Null)])));
    }

    #[test]
    fn number_and_version_ranges_observe_inclusive_boundaries() {
        let number = compile(
            json!({"/duration": {"min": 2, "max": 4, "include_max": false}}),
            &REQUEST_RULE_MATCH_SCHEMA,
        )
        .unwrap();
        assert!(number.matches(&context(&[("/duration", json!(2))])));
        assert!(!number.matches(&context(&[("/duration", json!(4))])));

        let version = compile(
            json!({"client_version": {"min": "2.2.0-beta.1", "max": "2.3.0", "include_max": false}}),
            &RELEASE_TRACK_MATCH_SCHEMA,
        )
        .unwrap();
        assert!(version.matches(&context(&[("client_version", json!("2.2.0"))])));
        assert!(!version.matches(&context(&[("client_version", json!("2.3.0"))])));
        assert!(!version.matches(&context(&[("client_version", json!("invalid"))])));
        assert_eq!(
            compile(
                json!({"client_version": {"min": "2.2.0+"}}),
                &RELEASE_TRACK_MATCH_SCHEMA,
            )
            .unwrap_err()
            .kind,
            MatchCompileErrorKind::InvalidRangeBound
        );

        let large_integer = compile(
            json!({"/tokens": {"min": 9007199254740993_u64}}),
            &REQUEST_RULE_MATCH_SCHEMA,
        )
        .unwrap();
        assert!(!large_integer.matches(&context(&[("/tokens", json!(9007199254740992_u64))])));
    }

    #[test]
    fn loading_rejects_unknown_dimensions_types_and_operators() {
        assert_eq!(
            compile(json!({"unknown": "*"}), &MODEL_DRIVER_MATCH_SCHEMA)
                .unwrap_err()
                .kind,
            MatchCompileErrorKind::UnknownDimension
        );
        assert_eq!(
            compile(json!({"family": 5}), &MODEL_DRIVER_MATCH_SCHEMA)
                .unwrap_err()
                .kind,
            MatchCompileErrorKind::InvalidValueType
        );
        assert_eq!(
            compile(
                json!({"family": {"glob": "gpt-*"}}),
                &MODEL_DRIVER_MATCH_SCHEMA
            )
            .unwrap_err()
            .kind,
            MatchCompileErrorKind::InvalidOperator
        );
        assert_eq!(
            compile(json!({"family": {"min": "a"}}), &MODEL_DRIVER_MATCH_SCHEMA)
                .unwrap_err()
                .kind,
            MatchCompileErrorKind::RangeNotAllowed
        );
        assert_eq!(
            compile(json!("high"), &REQUEST_RULE_MATCH_SCHEMA)
                .unwrap_err()
                .kind,
            MatchCompileErrorKind::ShorthandNotAllowed
        );
        assert_eq!(
            compile(json!({"quality": "high"}), &REQUEST_RULE_MATCH_SCHEMA)
                .unwrap_err()
                .kind,
            MatchCompileErrorKind::UnknownDimension
        );
        assert!(compile(
            json!({"/nested/~0key/~1value": true}),
            &REQUEST_RULE_MATCH_SCHEMA
        )
        .is_ok());
        assert_eq!(
            compile(json!({"/bad/~key": true}), &REQUEST_RULE_MATCH_SCHEMA)
                .unwrap_err()
                .kind,
            MatchCompileErrorKind::UnknownDimension
        );
    }

    #[test]
    fn match_rule_serde_round_trip_preserves_both_forms() {
        for value in [
            json!("gpt-*"),
            json!({"origin_model_id": "gpt-*", "api_type": ["llm"]}),
        ] {
            let rule: MatchRule = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(rule).unwrap(), value);
        }
        assert!(serde_json::from_value::<MatchRule>(json!([])).is_err());
        assert!(serde_json::from_value::<MatchRule>(json!(42)).is_err());
    }

    #[test]
    fn all_business_schemas_share_the_same_matcher_contract() {
        let cases = [
            (
                &MODEL_DRIVER_MATCH_SCHEMA,
                json!("gpt-*"),
                "origin_model_id",
            ),
            (
                &PROVIDER_RULE_MATCH_SCHEMA,
                json!("gpt-*"),
                "provider_model_id",
            ),
            (
                &REQUEST_RULE_MATCH_SCHEMA,
                json!({"/model": "gpt-*"}),
                "/model",
            ),
            (
                &PRICING_RULE_MATCH_SCHEMA,
                json!({"/model": "gpt-*"}),
                "/model",
            ),
            (
                &RELEASE_TRACK_MATCH_SCHEMA,
                json!("gpt-*"),
                "client_version",
            ),
        ];
        for (schema, rule, dimension) in cases {
            let compiled = compile(rule, schema).unwrap();
            assert!(compiled.matches(&context(&[(dimension, json!("gpt-5"))])));
            assert!(!compiled.matches(&context(&[(dimension, json!("claude-4"))])));
        }
    }

    #[test]
    fn ordered_rule_set_returns_non_sensitive_trace_metadata() {
        let rules = CompiledRuleSet::compile(
            [
                RuleEntry {
                    rule_id: Some("model-claude".to_owned()),
                    rule: serde_json::from_value(json!("claude-*")).unwrap(),
                },
                RuleEntry {
                    rule_id: Some("model-gpt".to_owned()),
                    rule: serde_json::from_value(json!({
                        "origin_model_id": "gpt-*",
                        "api_type": "llm"
                    }))
                    .unwrap(),
                },
            ],
            &MODEL_DRIVER_MATCH_SCHEMA,
        )
        .unwrap();
        let trace = rules
            .first_match(&context(&[
                ("origin_model_id", json!("gpt-secret-model-name")),
                ("api_type", json!("llm")),
            ]))
            .unwrap();
        assert_eq!(trace.rule_id.as_deref(), Some("model-gpt"));
        assert_eq!(trace.position, 1);
        assert_eq!(trace.dimensions, ["api_type", "origin_model_id"]);
        assert!(!format!("{trace:?}").contains("secret"));
    }

    #[test]
    fn routing_schemas_bind_their_declared_primary_dimensions() {
        let provider = compile(json!("primary-*"), &ROUTING_PROVIDER_MATCH_SCHEMA).unwrap();
        assert!(provider.matches(&context(&[("provider_instance_name", json!("primary-cn"))])));
        let model = compile(json!("gpt-5@primary"), &ROUTING_MODEL_MATCH_SCHEMA).unwrap();
        assert!(model.matches(&context(&[("exact_model", json!("gpt-5@primary"))])));
    }
}
