# AICC Model Driver Metadata Schema

Implemented contract: Model Driver **2.0**, 2026-09-25. This is a breaking change.
Provider Rules, Known Provider and the system-config envelope keep their existing
versions. Implementation, offline coverage and remaining Provider integration are
recorded in [the implementation report](model_driver_v2_implementation.md).

## LLM target contract: vendor specifications and model families

The implemented tree is `task -> vendor specification -> family:effort -> instance`.
Model Driver metadata describes official identities and capabilities. It neither
registers Providers nor promises that a channel can execute every supported effort.

### Document and model fields

```json
{
  "format": "buckyos.aicc.model-driver-catalog",
  "schema_version": 2,
  "schema_revision": 0,
  "model_driver_id": "openai",
  "revision_seq": 2,
  "required_features": [],
  "specs": [{ "id": "gpt-pro", "direct_only": true }],
  "models": [{
    "id": "gpt-5.6-sol",
    "api_types": ["llm"],
    "capabilities": { "tool_call": true, "max_context_tokens": 200000 },
    "llm": {
      "spec": "gpt-pro",
      "effort": "high",
      "default_effort": "medium",
      "supported_efforts": ["none", "low", "medium", "high", "xhigh"],
      "stability": "stable"
    }
  }],
  "patterns": [],
  "defaults": {}
}
```

This abbreviated example illustrates fields, not the complete capability table.
The reviewed [OpenAI builtin](../../src/frame/aicc/driver_metadata/models/openai.model.json)
is the complete repository fixture. Every origin vendor has one document; the
filename does not change the stable `model_driver_id` (Anthropic uses `claude`).

| Field | Contract |
| --- | --- |
| `specs[].id` | Unique normalized path segment; declares `llm.{id}` independently of inventory. |
| `specs[].direct_only` | Defaults to false. A specification must have a task reference or explicitly set this flag. Task/fallback references cannot bypass it. |
| `models[].id` | Official origin identity, preserved for matching. Provider channel IDs remain separate. |
| `llm.spec` | Exactly one specification declared by the same Model Driver. |
| `llm.family_id` | Optional normalized segment used only to resolve family naming conflicts. |
| `llm.effort` | Fixed effort of the specification's family reference. |
| `llm.default_effort` | Effort used when selecting the family without a suffix. |
| `llm.supported_efforts` | Nonempty, unique list containing both selected efforts. |
| `llm.stability` | Required `stable` or `experimental`. |

Efforts are `native`, `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`,
and `thinking`. Each non-native effort derives the semantic variant identity
`reasoning-{effort}`. `native` selects the base model without inventing a variant
or parameter. `thinking` describes a thinking toggle, not a numeric medium tier.
Supported effort facts are not Provider parameter templates.

Family paths normalize `.`, `/`, `_`, and `-` separators to lowercase hyphenated
segments. `gpt-5.6-sol` becomes `llm.gpt-5-6-sol`; the official ID stays unchanged.
Families, specifications and task names must not collide, even with no inventory.

### Matching and validation

Model Driver membership is a finite exact ID set. Literal patterns and finite
`origin_model_id` arrays expand into exact entries during compilation; explicit
`models[]` entries win. All wildcard model membership is rejected, including
non-LLM membership. Defaults only supply missing semantics to an existing exact
entry. `resolve_model(driver, model)` rejects unknown IDs.

Inventory identity is resolved across all loaded drivers: instance
`model_driver_overrides` → optional Provider matcher → exact ID → longest bounded
case-insensitive containment. A containment match must begin at a non-alphanumeric
boundary and end at the string end or a date-shaped suffix (`-YYYY-MM-DD`,
`-YYYYMMDD`, `-YYMMDD`, `-MMDD`). Equal longest candidates are ambiguous.
Provider failure and invalid overrides are terminal for that model. Unknown and
ambiguous IDs produce diagnostics and never enter the routable inventory.

Every effective, non-excluded LLM rule requires `llm`; non-LLM rules cannot carry
it. LLM ownership cannot redirect to a different driver. Duplicate specifications,
invalid efforts, undeclared membership, invalid normalized names and name collisions
fail catalog construction without Providers. Task references, `direct_only` and
graph cycles are validated when assembling the tree.

LLM rules cannot declare nonempty `logical_mounts`. Their admission belongs to
task-to-specification references. Independent non-LLM models retain mounts,
capabilities and canonical mappings. Non-LLM tasks served by an LLM explicitly
reference a specification in the builtin overlay; the leaf must support the
requested API. An LLM is never automatically admitted solely for its capabilities.

The parser rejects schema v1 and removed fields: Model Driver `variants`,
`version_rules`, `version_order`, `model_pricing` and inline `pricing`. No alias,
upgrade path, old-directory redirect or default-price slot remains. Pricing
structures shared with Provider Rules remain exclusively for that contract.
Missing channel price remains unknown, never zero or an origin price estimate.

### Version derivation and selection

Version extraction follows each known driver's official ID convention. It uses
up to three numeric release components, padded with zeros; dates, parameter
counts and product suffixes do not extend the release number. Both Claude
`claude-3-5-sonnet` and `claude-sonnet-4-6` naming orders are recognized.

Comparison uses numeric tuples: `5.6 < 5.10 < 6.0`. The optional decimal view
`major * 100 + minor * 10 + patch` applies only to single-digit minor/patch
components (`5.6 -> 560`, `5.6.1 -> 561`). Unknown versions sort after recognized
versions within their stability class. Equal versions use normalized family ID
ascending; declaration order does not affect the result.

After filtering task/request constraints, inventory and channel executability,
selection compares specification weight descending, specification ID ascending,
then stability, version descending and family ID ascending within that specification.
It does not multiply weights by versions or compare versions across specifications.
The existing scheduler compares physical instances within the chosen family.
Additional instances do not duplicate the specification reference or its weight.
Experimental families require the internal `RoutingRequest.allow_experimental`
policy (false by default); a qualified stable family in the same specification
always precedes them. No public RPC field was added in this change.

### Static and dynamic trees

`ModelRegistry::build` creates definitions and all declared specifications before
registering inventory. A model-only `CatalogDocuments` with empty Provider Rules
and Known Providers is valid. With inventory `[]`, all task/specification views
are queryable, references and weights remain visible, and exact candidates are
empty. No families or executable models are invented.

A family is materialized from the intersection of catalog identity and inventory.
Its candidates additionally intersect supported efforts with inventory variants.
A missing effort has no candidate; it does not substitute another effort or base
model. Removing the final instance removes its family and incoming dynamic edge,
while preserving tasks, specifications and task weights. Other channels retain
the same family when the original channel disappears.

`:high` is a virtual family selector, not a `.high` directory. A family without a
suffix uses `default_effort`; a specification fixes `effort`. Tree expansion and
request routing retain that selected exact variant. Final wire parameter locking
is separate Provider/Adapter work.

LLM directories require Manual admission. The `llm` root is a namespace and
`llm.fallback` starts empty. Tasks, specifications and families have no implicit
Parent fallback. Explicit fallback retains original task/request requirements.
Legacy `llm.{driver}.{model}` names are not aliases.

Overlays retain `factory -> system -> user -> session` precedence. Item and
fallback graphs are checked together for cycles. Rebuilds are deterministic;
RuntimeState publishes only complete valid snapshots and preserves the previous
snapshot on catalog or tree validation failure. Directory RPC shapes are unchanged.

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

For one origin model, the compiled exact entry overlays Model Driver defaults.
Rules from shadowed source documents do not participate.

## Provider integration boundary

The Provider upgrade is implemented separately from the v2 model kernel.
`static_inventory_models` is explicit channel inventory; neither Model Driver
membership nor Provider technical/pricing rules imply availability. Dynamic
success may only be supplemented for explicitly declared uncovered APIs via
`supplemental_inventory_api_types`. All inputs use the same identity resolver.

Executable presets are the intersection of `supported_efforts`, compiled Provider
mappings and observed channel restrictions. Inventory and lowering use the same
mapping lookup. `native` uses base; all other efforts use `reasoning-{effort}`,
including `reasoning-minimal` and `reasoning-thinking`. Invalid exact/cached
variants are rejected. A selected preset owns its thinking parameters after
canonical and request-rule rewrites.

Inventory schema 2 exposes `identity_source`, `inventory_source`,
`unmatched_models` and `unavailable_presets`. `list_providers` includes identity,
preset and unpriced diagnostics; refresh replies include `unmatched_count`.
Old inventory caches and removed Provider fields are rejected.

Pricing belongs to Provider Rules/discovery and supports `source_url`,
`verified_at`, `ratio_exception`, cache reads/writes (including one-hour writes),
audio/image token rates, tiers and time windows. Discovery and catalog share
validation. Missing active rates or uncovered tiers return unknown. `AiUsage`
input/output totals include cache/reasoning tokens; modality/cache counts are
subsets. Undeclared provider `reported_cost` is never assumed USD.

A Known Provider catalog may carry `exchange_rates` with `source_url`,
`observed_at_ms`, `expires_at_ms`, and `usd_per_unit`. Rates are usable only in the
half-open observation/expiry interval. Routing uses a 1,000 input / 1,000 output
reference (request estimates override it), the same tier/time-window resolver as
settlement, and valid USD conversions. Same-family, fixed-effort instances use
price first, then latency/reliability; unknown quotes are last and fail a hard
cost budget. Specification and version preferences still take precedence.

See [Provider upgrade implementation](provider_upgrade_implementation.md) for
verified sources, regional scope, remaining channel gaps and validation.
