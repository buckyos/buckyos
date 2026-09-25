# AICC Model Driver Metadata Schema

Model Driver metadata maps an origin model identity to stable model semantics:
API types, capabilities, logical mounts, family/version relationships and
semantic variants. Provider discovery supplies channel-local model IDs;
Provider Rules resolve those IDs to an origin identity before this metadata is
matched.

Provider-specific origin mappings, exclusions, operations, request rules and
endpoints are not valid Model Driver fields. They belong to the Provider Rules
catalog described by `provider_profile_schema.md`. Prices are channel facts:
only Provider Rules may declare a `model_pricing` table. Model Drivers must not
declare prices or supply fallback cost estimates. Missing reliable channel
prices remain unknown, not zero or an inferred origin-vendor price. In the LLM
target, `supported_efforts` declares model reasoning choices once; the Protocol
Adapter and Provider Rules translate them into request parameters. The v1 Model
Driver variant fallback described below is not part of that target.

All boolean matching uses the shared `MatchRule` defined by `match_rule.md`.
Simple model rules remain wildcard strings; the object form is only used when a
rule must constrain multiple dimensions.

## LLM target contract: vendor specifications and model families

Design update: 2026-09-25. This section follows the target contract in
[`service/model_defaults.rs`](../../src/frame/aicc/src/service/model_defaults.rs).
It defines the next LLM metadata structure; **the current parser still implements
schema v1 and does not support these fields**. The OpenAI builtin
JSON is now a v2 review draft; implementation awaits review approval. The sections
below this target contract describe the existing v1 schema. Dedicated non-LLM
models without specification membership retain their existing mount design.
The schema v2 example here is an excerpt from the target structure.

The pricing boundary applies to all models, including non-LLM models. All builtin
Model Driver JSON files now omit `model_pricing`. The existing v1 parser and
inventory builder still accept/use Model Driver prices; removing that support
and rejecting the field are pending implementation review. This document's
pricing boundary supersedes the former v1 Model Driver fallback.

### Ownership and identities

One origin vendor owns one Model Driver document, for example
[`models/openai.model.json`](../../src/frame/aicc/driver_metadata/models/openai.model.json).
It contains a small specification declaration and a much larger official-model
catalog. AICC has no universal five-level scale and must not infer specifications
from model-name suffixes, Provider names, protocol adapters or `parameter_scale`.

| Object | Identity and responsibility |
| --- | --- |
| Model Driver | `model_driver_id`, e.g. `openai`; owns official IDs, intrinsic capabilities, specifications and fixed reasoning presets. |
| Specification | `specs[].id`, e.g. `gpt-pro`; creates `llm.gpt-pro` even with no Provider inventory. The product-line prefix need not equal the driver ID. |
| Official model / family | `models[].id` is the unmodified official `origin_model_id`; its default family path is `llm.{normalized origin_model_id}`, e.g. `llm.gpt-5-6-sol`. |
| Family preset | `llm.gpt-5-6-sol:high`; a fixed reasoning choice, not a dot-separated child directory or another model generation. |
| Physical instance | Existing exact identity `<provider_model_id>[:<variant>]@<provider_instance_name>`; multiple instances of the same official model share the family and preset. |
| Task preference | Builtin/system/user/session logical overlay, e.g. `llm.plan -> llm.gpt-pro`; does not belong in the vendor's model facts. |

The GPT target grouping has five general specifications: `gpt-nano`, `gpt-mini`,
`gpt-standard`, `gpt-pro`, `gpt-max`. `gpt-codex` is an additional specialized
specification, not a sixth general level. These are AICC routing declarations
from the repository's design contract, not a claim about a vendor API enum.
Other vendors declare their own product lines and specialized specifications.

Family normalization reuses the mount-segment convention: lowercase and join
nonempty parts separated by `.`, `/`, `_` or `-` with `-`. Preserve the official
ID for matching and invocation. A normalized family ID must not collide with a
task, a specification or a different official model. An explicit `llm.family_id`
may resolve a naming collision; it is not a mechanism for merging unrelated
models. Two channels belong to one family only after Provider Rules/discovery
resolve them to the same official identity. Floating aliases require a confirmed,
Provider-specific underlying identity; never guess it from the latest version.

### Proposed schema v2 fields

`specs` declares LLM specifications independently of `models` and inventory.
Each entry has a globally unique, version-free `id` and a `direct_only` boolean
(default `false`). Every specification must either be referenced by a task or be
`direct_only`. All five GPT general specifications and `gpt-codex` have task
references in the target builtin tree. Do not store task weights in `specs`.

The catalog's main body is exact `models[]` entries containing official IDs,
`api_types`, intrinsic `capabilities` and the following `llm` object. Each
effective, non-excluded rule whose API types contain `llm` must resolve to exactly
one specification in the same Model Driver. Non-LLM rules do not require `llm`.

| `llm` field | Meaning |
| --- | --- |
| `family_id` | Optional single path segment; normally derived from the official model ID. |
| `spec` | Required reference to one `specs[].id` in this document. |
| `effort` | Required member of `supported_efforts`; fixes the reasoning strength used when entering this family through its specification. |
| `stability` | Required `stable` or `experimental`; preview/beta/exp releases must be declared explicitly. |
| `default_effort` | Required member of `supported_efforts`, used for direct family selection without an explicit `:effort`. It need not equal `effort`. |
| `supported_efforts` | Required nonempty list of supported effort names; the sole declaration of this model's reasoning choices. AICC derives `reasoning-{effort}` identities; `native` uses the base exact model without an effort parameter. No duplicate `variants` table is needed. |

Version ordering is derived from the official model ID, not written as
`version_order` in each model entry. For the current OpenAI release names, parse
the version immediately following `gpt-`, with missing minor/patch components
set to zero. For single-digit minor and patch components, the integer value is
`major * 100 + minor * 10 + patch`: `5.6 -> 560`, `5.5 -> 550`, `6 -> 600`,
and `5.6.1 -> 561`. Treat these as version components, not floating-point numbers.
If minor or patch components exceed one digit, compare the numeric component
tuples so `5.10` sorts after `5.6` and before `6.0`; do not use an overflowing
decimal slot calculation that collides with another version.

Derive only the release version according to the vendor's naming convention;
parameter counts, dates and product suffixes do not become version components.
Models without a recognizable release version have no inferred newer/older
relationship and use the same deterministic model-ID tie-break. They remain
eligible fallbacks after models with a recognized version in the same stability
class. A pattern must not assign an invented version to an unknown generation.

Equal versions are valid: `gpt-5.6` and `gpt-5.6-sol` both yield `560`, and the
normalized family ID in ascending order breaks ties. Do not create artificial
version differences, infer aliases or use JSON declaration order to decide the
winner. Stability and request eligibility are checked separately; version
comparison is confined to one specification and never changes task weights.

For example, the following excerpt expresses the header's `gpt-pro ->
gpt-5.6-sol:high` relationship. Capability values are taken from the existing
repository metadata, not newly verified vendor specifications. Other models and
capabilities are omitted; prices and protocol parameters do not belong here.

```json
{
  "format": "buckyos.aicc.model-driver-catalog",
  "schema_version": 2,
  "schema_revision": 0,
  "model_driver_id": "openai",
  "revision_seq": 2,
  "required_features": [],
  "specs": [
    { "id": "gpt-nano" },
    { "id": "gpt-mini" },
    { "id": "gpt-standard" },
    { "id": "gpt-pro" },
    { "id": "gpt-max" },
    { "id": "gpt-codex" }
  ],
  "models": [
    {
      "id": "gpt-5.6-sol",
      "api_types": ["llm"],
      "capabilities": {
        "streaming": true,
        "tool_call": true,
        "json_schema": true,
        "reasoning": true,
        "vision": true,
        "max_context_tokens": 1050000,
        "max_output_tokens": 128000
      },
      "llm": {
        "spec": "gpt-pro",
        "effort": "high",
        "stability": "stable",
        "default_effort": "high",
        "supported_efforts": ["high"]
      }
    }
  ],
  "patterns": [],
  "defaults": {}
}
```

For a model assigned to a specification, omit `logical_mounts` entirely, including
its former `vision.*`, `image.*` and `agent_runtime.*` paths. Specification
membership and the normalized official model ID determine its family/preset
relationships. Functional paths and their references belong to the common
logical tree; they are not repeated in each model entry. `api_types` and
`capabilities` constrain which requests an instance can execute, but do not
automatically attach it to every compatible task. In particular, membership in
`gpt-nano` alone does not attach a model to `image.txt2img` or `vision.ocr`;
those non-LLM entry points need their own explicit tree policy and API checks.
Dropping the old per-model paths does not implicitly preserve those old routes.

Dedicated non-LLM models such as `gpt-image-2`, embeddings, audio and video
models have no specification membership in this LLM design. Their existing
`logical_mounts` remain until their own grouping and tree references are defined;
do not replace them with universal automatic task admission.

The family directory is derived; no `llm.openai.{model}`, per-model task mounts,
or duplicate specification-to-model list is needed. `models[].llm` is the single
source of that relationship. LLM preset selectors replace the v1
`variants[].mount_suffix` expansion into `.reasoning-high` child directories;
the exact variant identity remains `:reasoning-high@provider`. Pricing belongs
only to Provider Rules or Provider discovery/responses, never to the Model Driver.
The large `models[]` body must enumerate supported official IDs; use
patterns only for confirmed equivalent forms, not to automatically admit future
generations. Preserve exact-first / first-pattern matching and validate the
resolved semantics. Enumerate known releases explicitly; deriving a version does
not itself admit a new model into a specification. Defaults must not assign unknown
models to an LLM specification.

Logical presets distinguish `none` (reasoning disabled), supported strengths such
as `low`/`medium`/`high`/`xhigh`, `thinking` (on/off-only model, enabled), and
`native` (non-adjustable behavior). The public fields use effort names directly;
the semantic variant is derived as `reasoning-{effort}` except for the native
base model. The model's `supported_efforts` is sufficient to define those
identities; do not repeat model lists or request parameter templates in Model
Driver `variants`. The target derives `reasoning-thinking` for an on/off switch
instead of labelling it `reasoning-medium`. `native` has no synthetic effort
variant. Runtime exact identities remain available for routing and audit.

The Protocol Adapter supplies standard effort-to-request conversion. For example,
the OpenAI Responses mapping takes the selected effort into `reasoning.effort`;
it does not need a per-model copy of this mapping in the Model Driver. Provider
Rules handle channel-specific mappings or restrictions. When Provider variant
rules match a model, their complete set is intersected with `supported_efforts`;
when none match, use the Adapter's supported standard mapping. Neither path may
invent an effort the model does not support. A missing Model Driver `variants`
table is not missing capability or a reason to reject the catalog.

A family preset accepts an instance only if that channel can execute its effort.
If neither the Adapter nor applicable Provider Rules can express it faithfully,
omit that candidate rather than fall back to the base model or another effort.
Provider mappings do not create family/specification memberships. Once selected
through a specification or fixed family preset, conflicting request reasoning
options must be rejected; neither merging nor later rewrites may replace the
effort. Derivation, Adapter conversion and the revised Provider resolution are
pending implementation; the v1 behavior and closed vocabulary below are unchanged.

### Tree construction, selection and validation

1. Load and validate the effective vendor catalogs. Create builtin task nodes
   and all declared specification nodes independently of inventory.
2. Intersect model definitions with effective Provider inventory. Create one
   family for each present official identity, attach executable fixed presets
   and exact instances, and add one specification-to-family-preset reference
   ordered by the derived model version. Provider count never multiplies that reference.
   Apply factory/system/user/session overlays in the existing order, validate
   the completed graph and publish atomically. Zero Providers still leaves
   task/specification nodes and task preferences visible.
3. Filter by the original task/request capabilities, inventory, Provider state
   and calling policy. Skip specifications with no eligible candidates, order
   specifications by task weight (ties by specification ID), then select the
   newest eligible stable family within the chosen specification. Experimental
   families are eligible only if policy allows them and no eligible stable
   family remains. Select physical instances within that family/preset using
   the Provider scheduler. Exhaust that specification before trying the next.
4. Task preference and version order are separate comparisons; do not multiply
   them or compare version orders across specifications. Family direct selection
   uses `default_effort` and is strict: it does not upgrade to another family.
   LLM root is namespace-only, `llm.fallback` is empty by default, and tasks,
   specifications and families have no implicit Parent fallback. An explicit
   fallback retains the original request and task constraints.
5. Rebuild the dynamic portion when inventory changes. Remove a family, its
   presets and incoming dynamic references when its last inventory instance
   disappears; keep tasks, empty specifications and their preference weights.
   Losing the official Provider does not remove instances on other Providers
   or delete model metadata. Temporary health filtering is not catalog removal.

Reject missing specification declarations, unused specifications without
`direct_only`, LLM rules without exactly one specification, missing/unsupported
efforts, naming collisions and cycles before publishing
a snapshot. Validate all LLM rule declarations even with zero inventory.
`direct_only` specifications cannot be reached implicitly through tasks or
fallback; an explicit task attachment must also clear that flag. All LLM task
paths receive models through specifications; `logical_mounts`, `auto_mounts`
and generic Auto/Hybrid admission cannot bypass this graph.

### Implementation boundary and acceptance

This is a breaking Model Driver schema change. Implement typed `specs`/`llm`
fields, reference validation and schema v2 gating together; do not silently load
them as v1 or provide a dual LLM routing path. Dedicated non-LLM models retain
their existing mounts. Non-LLM routes formerly mounted directly by specification
members must instead be reviewed as common-tree policy; preserving API capability
declarations alone does not preserve those routes. Catalog source priority,
complete-document replacement and overlay order remain.
Replace LLM `version_rules.current_mount` and `version_rules.auto_mounts` with
the declared relationships and derived version ordering. Migrate every builtin
LLM driver and its task references together; retaining v1 fields for non-LLM
rules does not authorize their use for LLMs. Invalid updates keep the current
snapshot.

Derive reasoning identities from each model's `supported_efforts` instead of
Model Driver `variants` and its fallback `provider_options`. Update inventory
expansion and Adapter/Provider effort conversion together. Other builtin drivers
still use v1 variant tables until their effort declarations are migrated; the
OpenAI v2 review draft already omits this redundant table. Do not copy the removed
per-model identity lists into Provider Rules just to preserve the duplication.

Remove Model Driver pricing from schema, compilation, inventory fallback and
price-source reporting. Provider discovery prices take precedence over applicable
Provider Rules prices; if neither exists, preserve unknown. Do not copy removed
origin-vendor prices into Provider Rules without confirming their channel and
billing conditions. A Provider's reported actual charge still takes precedence
for settlement. Rebuilt inventories must not retain old Model Driver prices.

Acceptance must cover zero-inventory specification visibility; dynamic family
creation and last-instance cleanup; multiple Providers with distinct channel IDs
sharing one family without inflating its version order; official-Provider removal
with another instance remaining; older eligible stable-version fallback; skipping
empty high-weight specifications; experimental-release gating; strict family
selection; unavailable and request-conflicting presets; `direct_only` isolation;
and each catalog validation failure above. Verify UI/`models.list` can distinguish
task, specification, family, preset and exact instance, including empty
specifications. No new public API fields are claimed implemented by this document.
Effort checks must cover a catalog without `variants`, exact identity derivation,
standard Adapter conversion without Provider variant rules, explicit channel
restrictions/mappings, unsupported-effort rejection and `native` without a
synthetic effort parameter.
Version-ordering checks must cover `5.6 -> 560`, missing components, numeric
multi-digit components, equal-version ties independent of declaration order,
unversioned models and excluding dates/parameter counts from the release version.
Pricing acceptance must cover rejecting `model_pricing` in a Model Driver,
unknown prices with no channel source, unchanged Provider Rules/discovery
precedence, and removal of stale Model Driver prices on inventory rebuild.
Missing prices must not become zero, inferred free service or a complete bill.

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
  JSON output, web search, vision, image generation and token limits.
- `canonical_fields`: maps canonical request JSON Pointers to mapping policies.
  Each policy names a converter implemented by AICC in Rust and configures how
  missing or unconvertible values are handled by one fallback policy. A Provider Rules entry
  with the same JSON Pointer replaces the complete Model Driver policy for that
  channel.
- scheduling hints: `estimated_latency_ms`, `quality_score`, `latency_class`
  and `cost_class`.

Neither inline `pricing` nor top-level `model_pricing` belongs in a Model Driver.
Qualitative scheduling hints are not a source of monetary prices or a fallback
for billing. Channel pricing and its matching rules are defined in
[`provider_profile_schema.md`](provider_profile_schema.md#55-pricing).

Provider Rules may only reduce the capabilities declared here; they cannot add
an intrinsic capability. Unknown models enter conservative fallback and do not
claim tool calling, JSON output, web search, vision or image generation.

## Variants

This section describes the current v1 implementation. The LLM target above
derives identities from `supported_efforts` and removes Model Driver variant
parameter templates; do not reintroduce this table into the v2 review draft.

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
