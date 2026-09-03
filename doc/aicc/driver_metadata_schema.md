# AICC Model Driver Metadata Schema

Model Driver metadata maps an origin model identity to stable model semantics:
API types, capabilities, logical mounts, family/version relationships and
semantic variants. Provider discovery supplies channel-local model IDs;
Provider Rules resolve those IDs to an origin identity before this metadata is
matched.

Provider-specific origin mappings, exclusions, operations, request fields,
endpoints and channel pricing are not valid Model Driver fields. They belong to
the Provider Rules catalog described by `provider_profile_schema.md`.

All boolean matching uses the shared `MatchRule` defined by `match_rule.md`.
Simple model rules remain wildcard strings; the object form is only used when a
rule must constrain multiple dimensions.

## Source priority

Metadata has four independent sources. From highest to lowest priority they are:

1. `$BUCKYOS_ROOT/etc/aicc/driver_metadata/system-config/`
2. `$BUCKYOS_ROOT/etc/aicc/driver_metadata/local/`
3. the current cloud source delivered and replaced by NDN
4. builtin metadata under `src/frame/aicc/driver_metadata/`

Selection is performed independently for each `(catalog_kind, catalog_id)`.
When the same identity exists in more than one source, the resolver selects the
highest-priority complete JSON document. It never merges fields, rules, arrays,
or defaults across source documents. A higher-priority source only shadows the
identities that it actually contains; it does not replace the effective catalog
set as a whole. For example, if cloud contains `openai.provider.json` but not
`minimax.provider.json`, cloud OpenAI and builtin MiniMax are both effective.

The effective catalog set is the union of these per-identity winners. Only after
this source-selection step does AICC validate references and build the immutable
catalog snapshot. `models/`, `providers/`, and `known-providers/` use the same
selection rule. A Known Provider file is atomic by `catalog_id`; independently
overridable providers therefore need independently stable catalog IDs/files.

All model parameters belonging to one origin vendor must be collected in that
vendor's single standalone file. The lowercase vendor slug is stable and must
not be split by model, API generation, or Provider Instance. Provider-vendor
parameters, including both origin vendors and aggregators, use one
`<provider-vendor-lowercase>.provider.json` file per vendor under a separate
`providers/` directory as specified by `provider_profile_schema.md`. Runtime
code must not compensate for missing metadata by branching on model names,
model-name prefixes, or Provider-vendor names.

For one origin model, match priority inside the selected Model Driver document
is exact `models[].id`, ordered `patterns[].match`, `defaults`, then conservative
fallback. Exact rules win before patterns; rules from shadowed source documents
do not participate.

## Document

```json
{
  "format": "buckyos.aicc.model-driver-catalog",
  "schema_version": 1,
  "schema_revision": 0,
  "model_driver_id": "openai",
  "revision_seq": 1,
  "required_features": [],
  "models": [],
  "patterns": [],
  "defaults": {},
  "variants": [],
  "version_rules": []
}
```

`model_driver_id` is the semantic driver identity, such as `openai`, `claude`,
`google-gemini`, `fal`, or `minimax`. `openrouter` is a Provider Profile and is
therefore not a Model Driver.

NDN must deliver a complete, internally consistent cloud-source file set
conforming to this schema. It need not duplicate identities supplied by builtin
or higher-priority sources. AICC resolves all four sources before parsing the
effective documents into runtime types. A cloud-source parse failure is an NDN
delivery-contract violation and must keep the update marker for diagnosis;
invalid local or system-config documents are reported against their own source
and must not produce a partially merged snapshot. The former `provider_driver`, `provider_options`,
`origin_provider_aliases`, `origin_mappings` and `signature` fields are rejected
in beta 2.2; no compatibility alias is provided. Catalog authenticity comes
from NDN's file delivery contract; AICC does not repeat file verification.

## Model rule

Exact and pattern rules can define the following fields. A pattern entry uses `match: MatchRule`;
for the common case it is only `"match": "gpt-*"`:

- `model_driver`: an exceptional semantic attribution override.
- `exclude`: excludes an origin model from this Model Driver.
- `parameter_scale`: display/classification metadata.
- `api_types`: intrinsic AICC API types.
- `logical_mounts`: semantic mounts using `{driver}` and `{model}` templates.
- `capabilities`: intrinsic capability limits such as streaming, tool calling,
  JSON output, web search, vision, image generation and token limits.
- `pricing`: last-resort semantic estimate only. Provider discovery, Provider
  Instance overrides and Provider Rules take precedence.
- scheduling hints: `estimated_latency_ms`, `quality_score`, `latency_class`
  and `cost_class`.

Provider Rules may only reduce the capabilities declared here; they cannot add
an intrinsic capability. Unknown models enter conservative fallback and do not
claim tool calling, JSON output, web search, vision or image generation.

## Variants

Variants define semantic identities only:

```json
{
  "name": "reasoning.high",
  "match": "gpt-*",
  "mount_suffix": "reasoning-high"
}
```

For `gpt-5.1`, this creates
`gpt-5.1:reasoning-high@<provider-instance>` and corresponding semantic mount
suffixes. The variant still calls the base channel model. A Provider Rules
entry matching `*:reasoning-high` converts that identity to protocol-specific
request options. Model Driver variants cannot contain `provider_options`.

## Version rules

`version_rules` select stable/current family mounts from a complete inventory
snapshot. They may match model patterns and tiers, rank versions, suppress
unstable or snapshot aliases, and attach semantic family mounts. These rules
do not select Provider operations or endpoints.
