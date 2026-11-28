use super::CodecRegistry;
use buckyos_api::ApiType;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// One protocol client operation: the unit a model rule binds an API type to.
/// Written `<protocol_adapter_id>/<operation_id>`; `/` is valid in neither ID.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct BindingRef {
    pub protocol_adapter_id: String,
    pub operation_id: String,
}

impl BindingRef {
    pub(crate) fn new(
        protocol_adapter_id: impl Into<String>,
        operation_id: impl Into<String>,
    ) -> Self {
        Self {
            protocol_adapter_id: protocol_adapter_id.into(),
            operation_id: operation_id.into(),
        }
    }

    /// A qualified value names its client; a bare operation belongs to
    /// `default_adapter_id`.
    pub(crate) fn parse(value: &str, default_adapter_id: &str) -> Self {
        match value.split_once('/') {
            Some((adapter, operation)) => Self::new(adapter, operation),
            None => Self::new(default_adapter_id, value),
        }
    }

    pub(crate) fn parse_qualified(value: &str) -> Option<Self> {
        let (adapter, operation) = value.split_once('/')?;
        (!adapter.is_empty() && !operation.is_empty() && !operation.contains('/'))
            .then(|| Self::new(adapter, operation))
    }

    pub(crate) fn qualified(&self) -> String {
        format!("{}/{}", self.protocol_adapter_id, self.operation_id)
    }
}

impl fmt::Display for BindingRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}/{}",
            self.protocol_adapter_id, self.operation_id
        )
    }
}

/// The protocol clients one Provider instance may use: its default client
/// first, then the extra clients in declared order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProtocolClientSet {
    pub default: String,
    pub extras: Vec<String>,
}

impl ProtocolClientSet {
    pub(crate) fn single(default: impl Into<String>) -> Self {
        Self {
            default: default.into(),
            extras: Vec::new(),
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.default.as_str()).chain(self.extras.iter().map(String::as_str))
    }

    pub(crate) fn contains(&self, protocol_adapter_id: &str) -> bool {
        self.iter().any(|id| id == protocol_adapter_id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BindingResolveError {
    UnknownAdapter(String),
    InvalidDefault {
        binding: BindingRef,
        api_type: String,
    },
    Ambiguous {
        protocol_adapter_id: String,
        api_type: String,
        candidates: Vec<String>,
    },
}

impl fmt::Display for BindingResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownAdapter(adapter) => {
                write!(formatter, "protocol adapter {adapter:?} is not registered")
            }
            Self::InvalidDefault { binding, api_type } => write!(
                formatter,
                "default operation {:?} does not support api_type {api_type}",
                binding.qualified()
            ),
            Self::Ambiguous {
                protocol_adapter_id,
                api_type,
                candidates,
            } => write!(
                formatter,
                "adapter {protocol_adapter_id:?} has multiple default operations {candidates:?} for api_type `{api_type}`"
            ),
        }
    }
}

pub(crate) struct BindingQuery<'a> {
    pub clients: &'a ProtocolClientSet,
    /// The rule-level client; the instance default applies when absent.
    pub rule_adapter_id: Option<&'a str>,
    pub overrides: &'a BTreeMap<String, String>,
    pub remote_methods: Option<&'a BTreeSet<String>>,
    pub api_type: ApiType,
    /// The operation the protocol default rules name, used to break a tie.
    pub default_operation: Option<&'a str>,
}

impl BindingQuery<'_> {
    pub(crate) fn primary_adapter_id(&self) -> &str {
        self.rule_adapter_id.unwrap_or(&self.clients.default)
    }
}

pub(crate) fn api_type_name(api_type: ApiType) -> String {
    serde_json::to_value(api_type)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("API types serialize as strings")
}

/// Resolves which client operation serves `api_type`. An `operations` entry
/// wins; otherwise the highest positive `default_priority` across the
/// instance's clients, preferring the primary client on a tie, and a binding
/// of priority 0 is only chosen when it is the primary client's only one.
pub(crate) fn resolve_binding(
    codecs: &CodecRegistry,
    query: &BindingQuery<'_>,
) -> Result<Option<BindingRef>, BindingResolveError> {
    let api_name = api_type_name(query.api_type);
    let primary = query.primary_adapter_id();
    if let Some(operation) = query
        .overrides
        .get(query.api_type.typed_method())
        .or_else(|| query.overrides.get(&api_name))
    {
        return Ok(Some(BindingRef::parse(operation, primary)));
    }
    let default_binding = query
        .default_operation
        .map(|operation| BindingRef::parse(operation, primary));
    if let Some(binding) = &default_binding {
        let supported = codecs
            .adapter(&binding.protocol_adapter_id)
            .and_then(|adapter| adapter.operations.get(&binding.operation_id))
            .is_some_and(|operation| {
                operation
                    .bindings
                    .iter()
                    .any(|candidate| candidate.api_type == query.api_type)
            });
        if !supported {
            return Err(BindingResolveError::InvalidDefault {
                binding: binding.clone(),
                api_type: api_name,
            });
        }
    }
    let mut order = vec![primary];
    order.extend(query.clients.iter().filter(|id| *id != primary));
    let mut candidates = Vec::new();
    for adapter_id in order {
        let adapter = codecs
            .adapter(adapter_id)
            .ok_or_else(|| BindingResolveError::UnknownAdapter(adapter_id.to_owned()))?;
        for operation in adapter.operations.values() {
            let Some(priority) = operation
                .bindings
                .iter()
                .filter(|binding| binding.api_type == query.api_type)
                .map(|binding| binding.default_priority)
                .max()
            else {
                continue;
            };
            candidates.push((
                BindingRef::new(adapter_id, &operation.operation_id),
                priority,
                adapter_id == primary,
            ));
        }
    }
    if let Some(methods) = query.remote_methods {
        let exact = candidates
            .iter()
            .filter(|(binding, _, _)| methods.contains(&binding.operation_id))
            .cloned()
            .collect::<Vec<_>>();
        candidates = if !exact.is_empty() {
            exact
        } else if methods.contains(query.api_type.typed_method()) || methods.contains(&api_name) {
            candidates
        } else {
            Vec::new()
        };
    }
    let highest = candidates
        .iter()
        .map(|(_, priority, _)| *priority)
        .max()
        .unwrap_or_default();
    let mut pool = if highest > 0 {
        let top = candidates
            .into_iter()
            .filter(|(_, priority, _)| *priority == highest)
            .collect::<Vec<_>>();
        if top.iter().any(|(_, _, is_primary)| *is_primary) {
            top.into_iter()
                .filter(|(_, _, is_primary)| *is_primary)
                .collect()
        } else {
            top
        }
    } else {
        candidates
            .into_iter()
            .filter(|(_, _, is_primary)| *is_primary)
            .collect::<Vec<_>>()
    };
    if pool.len() > 1 {
        if let Some(default) = default_binding
            .as_ref()
            .filter(|default| pool.iter().any(|(binding, _, _)| binding == *default))
        {
            return Ok(Some(default.clone()));
        }
    }
    match pool.len() {
        0 => Ok(None),
        1 => Ok(Some(pool.remove(0).0)),
        _ => Err(BindingResolveError::Ambiguous {
            protocol_adapter_id: primary.to_owned(),
            api_type: api_name,
            candidates: pool
                .into_iter()
                .map(|(binding, _, _)| {
                    if binding.protocol_adapter_id == primary {
                        binding.operation_id
                    } else {
                        binding.qualified()
                    }
                })
                .collect(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codecs() -> std::sync::Arc<CodecRegistry> {
        crate::provider::builtin_provider_codecs().unwrap()
    }

    fn resolve(
        clients: &ProtocolClientSet,
        rule_adapter_id: Option<&str>,
        overrides: &[(&str, &str)],
        api_type: ApiType,
    ) -> Result<Option<BindingRef>, BindingResolveError> {
        let overrides = overrides
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect::<BTreeMap<_, _>>();
        resolve_binding(
            &codecs(),
            &BindingQuery {
                clients,
                rule_adapter_id,
                overrides: &overrides,
                remote_methods: None,
                api_type,
                default_operation: None,
            },
        )
    }

    #[test]
    fn qualified_values_round_trip_and_bare_values_take_the_default_client() {
        let binding = BindingRef::parse("openai-images/images.generate", "openai-responses");
        assert_eq!(binding, BindingRef::new("openai-images", "images.generate"));
        assert_eq!(binding.qualified(), "openai-images/images.generate");
        assert_eq!(
            BindingRef::parse("responses.create", "openai-responses"),
            BindingRef::new("openai-responses", "responses.create")
        );
        for invalid in ["/op", "client/", "a/b/c", "op"] {
            assert!(BindingRef::parse_qualified(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn mappings_resolve_qualified_then_rule_client_then_instance_default() {
        let clients = ProtocolClientSet {
            default: "openai-responses".into(),
            extras: vec!["openai-chat-completions".into()],
        };
        assert_eq!(
            resolve(
                &clients,
                None,
                &[("llm", "openai-chat-completions/chat.completions.create")],
                ApiType::Llm
            )
            .unwrap(),
            Some(BindingRef::new(
                "openai-chat-completions",
                "chat.completions.create"
            ))
        );
        assert_eq!(
            resolve(
                &clients,
                Some("openai-chat-completions"),
                &[("llm", "chat.completions.create")],
                ApiType::Llm
            )
            .unwrap(),
            Some(BindingRef::new(
                "openai-chat-completions",
                "chat.completions.create"
            ))
        );
        assert_eq!(
            resolve(&clients, Some("openai-chat-completions"), &[], ApiType::Llm).unwrap(),
            Some(BindingRef::new(
                "openai-chat-completions",
                "chat.completions.create"
            ))
        );
        assert_eq!(
            resolve(&clients, None, &[], ApiType::Llm).unwrap(),
            Some(BindingRef::new("openai-responses", "responses.create"))
        );
    }

    #[test]
    fn other_clients_only_serve_by_positive_priority_and_ties_are_rejected() {
        let clients = ProtocolClientSet {
            default: "claude-messages".into(),
            extras: vec!["gemini-generate-content".into()],
        };
        assert_eq!(
            resolve(&clients, None, &[], ApiType::VideoGenerate).unwrap(),
            Some(BindingRef::new(
                "gemini-generate-content",
                "models.predictLongRunning"
            ))
        );
        let chat_only = ProtocolClientSet {
            default: "claude-messages".into(),
            extras: vec!["openai-chat-completions".into()],
        };
        assert_eq!(
            resolve(&chat_only, None, &[], ApiType::Llm).unwrap(),
            Some(BindingRef::new("claude-messages", "messages.create"))
        );
        assert_eq!(
            resolve(&chat_only, None, &[], ApiType::EmbeddingText).unwrap(),
            None
        );
        assert!(matches!(
            resolve(
                &ProtocolClientSet::single("gemini-generate-content"),
                None,
                &[],
                ApiType::EmbeddingText
            ),
            Ok(Some(_))
        ));
    }
}
