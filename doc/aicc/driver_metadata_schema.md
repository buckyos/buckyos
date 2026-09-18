# AICC Model Driver Metadata Schema

Model Driver metadata maps an origin model identity to stable model semantics:
API types, capabilities, logical mounts, family/version relationships and
semantic variants. Provider discovery supplies channel-local model IDs;
Provider Rules resolve those IDs to an origin identity before this metadata is
matched.

Provider-specific origin mappings, exclusions, operations, request rules and
endpoints are not valid Model Driver fields. They belong to the Provider Rules
catalog described by `provider_profile_schema.md`. Channel prices are declared
independently of the technical rules, in the `model_pricing` table shared by both
catalogs. A Model
Driver variant may carry fallback `provider_options`; these defaults are used
only when the selected Provider Rules has no variant matching that concrete
model.

All boolean matching uses the shared `MatchRule` defined by `match_rule.md`.
Simple model rules remain wildcard strings; the object form is only used when a
rule must constrain multiple dimensions.

## Source priority

Metadata has four independent sources. From highest to lowest priority they are:

1. system-config key `services/aicc/driver_metadata`
2. `$BUCKYOS_ROOT/etc/aicc/driver_metadata/local/`
3. the current cloud source delivered and replaced by NDN
4. builtin metadata compiled into AICC by the metadata source manager

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

## Production source loading

The builtin source has no production runtime path. Its source files are kept in
exactly these three development directories and are compiled into AICC once by
the metadata source manager using `include_bytes!` or `include_str!`:

```text
src/frame/aicc/driver_metadata/models/
src/frame/aicc/driver_metadata/providers/
src/frame/aicc/driver_metadata/known-providers/
```

The metadata source manager owns the complete embedded file list, parses every document as
`MetadataSource::Builtin`, and rejects the complete builtin set when a document
is malformed or a catalog identity is duplicated. Provider-specific modules
must not embed, enumerate or expose their own copies of these files.

The local source enumerates direct `*.json` files from exactly three directories:

```text
$BUCKYOS_ROOT/etc/aicc/driver_metadata/local/models/
$BUCKYOS_ROOT/etc/aicc/driver_metadata/local/providers/
$BUCKYOS_ROOT/etc/aicc/driver_metadata/local/known-providers/
```

The directory name determines the catalog kind. Nested directories, symlinks and
non-JSON entries are rejected. A reload reads the complete tree twice and accepts
it only when both reads are identical. Its opaque revision is the SHA-256 digest
of the sorted relative paths and file contents.

The system-config source is one atomic value at key
`services/aicc/driver_metadata`:

```json
{
  "schema_version": 1,
  "model_drivers": [],
  "provider_rules": [],
  "known_providers": []
}
```

Each array contains complete catalog documents of the corresponding kind. The
system-config key revision is the source revision; using one key prevents a
reload from observing a mixture of independently updated documents. A missing
key means an empty system-config source at revision `0`.

`service.reload_settings` captures a fresh local content revision and a fresh
system-config key revision while preparing its invisible runtime candidate. Any
I/O error, malformed document, unsupported envelope version or source change
rejects the candidate and preserves the currently published RuntimeSnapshot.
Cloud `metadata_target_seq` remains independent of these two revisions.

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
  "schema_revision": 1,
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
and must not produce a partially merged snapshot. The former root-level `provider_driver`, `provider_options`,
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
  JSON output, web search, vision, audio input, image generation and token
  limits.
  A model whose transport AICC does not implement yet is booked here with
  `exclude: true` instead of being dropped from the catalog:

  ```json
  {
    "id": "glm-realtime-flash",
    "exclude": true
  }
  ```

  The model keeps its identity and its `model_pricing` entry, but it never
  reaches an inventory and is never routed. There is no separate "not wired
  yet" flag: `exclude` already states that the model is not served, and a
  second flag would only be one more thing to clear at wiring time. The
  Provider Rules `models[]` still needs the matching `exclude` entry, because
  the static fallback inventory is assembled from both sides. Wiring the
  protocol later therefore means: add the Provider Rules `operations` mapping
  and drop the `exclude` entries on both sides.
- `canonical_fields`: maps canonical request JSON Pointers to mapping policies.
  Each policy names a converter implemented by AICC in Rust and configures how
  missing or unconvertible values are handled by one fallback policy. A Provider Rules entry
  with the same JSON Pointer replaces the complete Model Driver policy for that
  channel.
- `pricing`: not a field of `models[]` or `patterns[]`. Prices live in the
  top-level `model_pricing` table, each entry keyed by `id` (exact origin model
  name) or `match` (wildcard). An exact `id` always wins; otherwise the first
  matching wildcard applies. The lookup is independent of whether the concrete
  model matched an exact entry or a pattern.
- scheduling hints: `estimated_latency_ms`, `quality_score`, `latency_class`
  and `cost_class`.

Provider Rules may only reduce the capabilities declared here; they cannot add
an intrinsic capability. Unknown models enter conservative fallback and do not
claim tool calling, JSON output, web search, vision, audio input or image
generation.

## Variants

Variants define semantic identities and their origin-provider fallback lowering:

```json
{
  "name": "reasoning-high",
  "match": "gpt-*",
  "mount_suffix": "reasoning-high",
  "provider_options": {
    "reasoning": {
      "effort": "high"
    }
  }
}
```

For `gpt-5.1`, this creates
`gpt-5.1:reasoning-high@<provider-instance>` and corresponding semantic mount
suffixes. The variant still calls the base channel model. A Provider Rules
entry matching `*:reasoning-high` converts that identity to protocol-specific
request options. Model Driver `provider_options` are the default lowering used
when the selected Provider has no matching variant. This field is available
from `schema_revision: 1`; revision 0 documents carrying it are rejected. The
same variant name may appear in multiple entries with disjoint model matches
when protocol parameters differ by model generation.

Variant resolution is model-specific and Provider-first. AICC first matches the
concrete `provider_model_id` against the selected Provider Rules `variants`.
If at least one Provider variant matches, those matches are the complete
effective variant set for that model. Model Driver `variants` are used only
when no Provider variant matches the model. The two sources are not merged or
deduplicated by a static variant identity key.

### Variant naming

Variant names are a closed, Provider-independent vocabulary shared by Model
Driver and Provider Rules catalogs. Every Model Driver `variants[].name` and
every Provider Rules `variants[].variant` MUST be exactly one of the following
tiers, ordered from no reasoning to the largest budget:

| Canonical name | Tier |
| --- | --- |
| `reasoning-none` | Reasoning disabled. |
| `reasoning-mini` | Lowest enabled tier; vendor `minimal`. |
| `reasoning-low` | Low. |
| `reasoning-medium` | Common/default tier; vendor `medium`/`normal`, or `enable` for a plain on/off switch. |
| `reasoning-high` | High. |
| `reasoning-xhigh` | Extra high; vendor `xhigh`. |
| `reasoning-max` | Maximum. |

Vendor-specific tier names MUST be normalized to the closest canonical name.
Names such as `effort-low`, `effort-medium`, `effort-high`, `effort-xhigh`,
`effort-max`, `thinking-enabled`, `thinking-disabled` or `reasoning-minimal`
are forbidden. Only the variant name is normalized: `provider_options` keep the
vendor's own parameter names and values.

Mapping rules:

- Vendor `disable` / `disabled` maps to `reasoning-none`.
- A model that only exposes an on/off switch maps `enable` to `reasoning-medium`.
- Vendor `medium` / `normal` or an equivalent common/default tier maps to
  `reasoning-medium`; tiers above or below map to the nearest canonical name
  (`minimal` -> `reasoning-mini`, `xhigh` -> `reasoning-xhigh`).
- A Model Driver variant and the Provider Rules variant that lowers the same
  concrete model MUST use the identical canonical name. `mount_suffix`, when
  present, uses that same string.

## Version rules

`version_rules` select stable/current family mounts from a complete inventory
snapshot. They may match model patterns and tiers, rank versions, suppress
unstable or snapshot aliases, and attach semantic family mounts. These rules
do not select Provider operations or endpoints.

| Field | Required | Meaning |
| --- | --- | --- |
| `id` | yes | Rule identity referenced by model rules/defaults. |
| `family` | yes | Family label used for diagnostics and grouping. It is not a match input. |
| `tier` | yes | Tier label used for diagnostics and grouping. It is not a match input. |
| `match` | yes | Matches `origin_model_id`; string shorthand is a glob. |
| `tier_tokens` | no | Tokens that all must occur in the normalized origin model ID. |
| `exclude_tier_tokens` | no | Tokens that disqualify a model from this tier. |
| `version_rank.prefix` | no | Prefix removed before numeric/version ranking. |
| `stability.unstable_tokens` | no | Tokens that mark a version as unstable. |
| `stability.current_requires_stable` | no | Prevents an unstable winner from receiving `current_mount`. |
| `current_mount` | yes | Mount assigned only to the highest ranked eligible model. |
| `version_mount` | yes | Mount assigned to every eligible version; `{model}` expands from `origin_model_id`. |
| `auto_mounts` | no | Additional mounts assigned to every eligible version. |

All model `logical_mounts` expand `{driver}` from `model_driver_id` and `{model}`
from `origin_model_id` before validation. Any remaining brace is rejected. Version
selection and ranking always use `origin_model_id`, so an aggregator's channel name
cannot alter the logical directory.
