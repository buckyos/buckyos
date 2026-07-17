# AICC Driver Metadata Schema

Driver metadata turns provider-discovered model ids into AICC `ModelMetadata`.
Provider `/models` discovery is only the id source; the resolver owns capability,
API type, mount, cost, latency, and conservative fallback decisions.

## Source Priority

The resolver loads metadata in this override order:

1. builtin metadata bundled under `src/frame/aicc/driver_metadata/`
2. remote cache: `$BUCKYOS_ROOT/etc/aicc/driver_metadata/remote_cache/<driver>.json`
3. local override: `$BUCKYOS_ROOT/etc/aicc/driver_metadata/local/<driver>.json`
4. system-config override materialized at `$BUCKYOS_ROOT/etc/aicc/driver_metadata/system-config/<driver>.json`

For each model id, match priority is:

1. exact `models[].id`
2. wildcard `patterns[].pattern`
3. `defaults`
4. conservative fallback

Exact matches win before patterns, even if the pattern comes from a higher
priority override.

## Document

```json
{
  "schema_version": 2,
  "provider_driver": "openai",
  "name": "OpenAI",
  "protocol_family": "openai-compatible",
  "base_url": null,
  "revision": "builtin-2026-05-30",
  "origin_provider_aliases": {},
  "origin_mappings": [],
  "models": [],
  "patterns": [],
  "defaults": {},
  "variants": [],
  "version_rules": [],
  "signature": null
}
```

Fields:

- `schema_version`: currently `2`.
- `provider_driver`: unique driver metadata id. For cloud-authored providers it
  is generated from the lowercase provider `name`, is used as the delivery JSON
  filename, and is written unchanged to this field.
- `name`: provider display name. In the Add Provider wizard it is user-entered,
  unique ignoring case among providers, and limited to English letters, digits,
  underscores, and hyphens.
- `protocol_family`: client-to-provider wire protocol family. Clients use it to
  choose how to communicate with the model vendor server. Supported values are
  currently `openai-compatible`, `anthropic`, `google-gemini`, `fal`, and
  `minimax`.
- `base_url`: optional endpoint key used to distinguish compatible providers.
  When omitted, clients use the default endpoint for `provider_driver`.
- `revision`: monotonically changing metadata revision string.
- `origin_provider_aliases`: optional map from provider-specific origin slugs to
  BuckyOS canonical origin driver names. This is normally used by aggregator
  providers such as OpenRouter.
- `origin_mappings`: optional ordered rules that derive the origin `driver` and
  origin `model` from the provider-native `provider_model_id`.
- `models`: exact rules keyed by `id`.
- `patterns`: wildcard rules keyed by `pattern`; `*` is the only wildcard.
- `defaults`: default rule when no exact or pattern rule matches.
- `variants`: optional provider option variants. The resolver expands each
  matching base model into additional AICC exact models whose provider model id
  is `<base>:<mount_suffix>`, while provider calls are lowered back to the base
  provider model plus `provider_options`.
- `version_rules`: optional post rules used to choose current family mounts,
  version index mounts, stability filtering, and auto mounts for matched model
  families.
- `signature`: optional signature envelope; verification is not enforced yet.

## Rule

Rules support these fields:

- `id`: exact provider model id for `models`.
- `pattern`: wildcard provider model id pattern for `patterns`.
- `model_driver`: optional per-model metadata driver. Defaults to the
  provider driver, but proxy/aggregator providers can override it when a model
  should be attributed to its upstream project or publisher. Aggregators such
  as OpenRouter or FAL should preserve upstream families here, for example
  `openai`, `claude`, `google-gemini`, `real-esrgan`, or `rembg`, instead of
  collapsing all models to the aggregator provider driver.
- `exclude`: valid on exact `models[]` and wildcard `patterns[]` rules. When true,
  the matching provider model is dropped from inventory; other parameter fields
  on the same rule are ignored for the current resolution but may remain in the
  document for later restoration. `defaults` does not use `exclude`.
- `parameter_scale`: optional display/classification string.
- `api_types`: AICC API types, for example `llm.chat`, `image.txt2img`, `audio.asr`.
- `logical_mounts`: logical mounts. Templates `{driver}`, `{model}`,
  `{provider_driver}`, and `{provider_model_id}` are expanded by the resolver.
  In schema v2, `{driver}` and `{model}` mean the resolved physical origin
  driver and origin model, not the current delivery channel.
- `capabilities`: partial capability patch: `streaming`, `tool_call`, `json_schema`, `web_search`, `vision`, `max_context_tokens`, `max_output_tokens`.
- `context_limits`: explicit model limit object, currently
  `max_context_tokens` and `max_output_tokens`. If omitted, clients and the
  resolver use configured defaults or conservative fallback.
- `pricing`: reference model price object with `price`, `currency`, and optional
  `unit`. If the provider service can return live pricing, provider live pricing
  takes precedence. Clients handle currency and exchange-rate conversion through
  their own shared currency pipeline.
- `estimated_cost_usd`, `estimated_latency_ms`: default scheduler estimates.
- `quality_score`, `latency_class`, `cost_class`: routing attributes.

Unknown fallback is intentionally conservative: it does not declare
`tool_call`, `web_search`, `vision`, or `json_schema`.

## Origin Identity Mappings

Provider model ids are channel-local names. For origin providers such as OpenAI,
`provider_driver = openai` and `provider_model_id = gpt-5.5` already identify the
physical model well enough. For aggregator providers such as OpenRouter, a
provider model id such as `openai/gpt-5.5` must be mapped back to the physical
origin identity before logical mounts are expanded.

Schema v2 defines these mount template variables:

| Variable | Meaning |
| --- | --- |
| `{driver}` | resolved physical origin provider, for example `openai` |
| `{model}` | resolved physical origin model name, for example `gpt-5.5` |
| `{provider_driver}` | current channel driver, for example `openrouter` |
| `{provider_model_id}` | current channel model id, for example `openai/gpt-5.5` |

`origin_mappings` are evaluated by ascending `priority`. The first successful
match wins. If no rule matches, the resolver falls back to:

```text
driver = provider_driver
model = provider_model_id
```

Mapping regexes are standard regular expressions. They must use named capture
groups `(?<driver>...)` and `(?<model>...)`; do not introduce custom regex
template syntax. Regex source is currently `provider_model_id`.

```json
{
  "origin_provider_aliases": {
    "google-gemini": "google",
    "x-ai": "xai"
  },
  "origin_mappings": [
    {
      "mapping_key": "openrouter-path-id",
      "priority": 100,
      "match": {
        "source": "provider_model_id",
        "regex": "^(?<driver>[^/]+)/(?<model>.+)$"
      },
      "transforms": {
        "driver": [
          { "op": "alias", "table": "origin_provider_aliases", "on_missing": "keep" }
        ],
        "model": [
          { "op": "trim" }
        ]
      }
    }
  ]
}
```

Supported transforms are intentionally limited and deterministic:

- `trim`: trim leading and trailing whitespace.
- `lowercase`: convert to lowercase.
- `alias`: look up the current value in the named alias table. `on_missing`
  defaults to `keep`; other resolver implementations may support `empty` or
  `error`.

For `provider_model_id = "openai/gpt-5.5"`, the example mapping resolves:

```text
driver = openai
model = gpt-5.5
```

Then `llm.{driver}.{model}` expands to `llm.openai.gpt-5-5`. An OpenAI origin
provider with `provider_model_id = "gpt-5.5"` also expands to the same logical
path through the fallback identity.

Dynamic provider aliases such as `~x-ai/grok-latest` or `openrouter/auto` are
not modeled in schema v2. They are provider-side soft links and should be
excluded with ordinary exact or pattern rules:

```json
{ "pattern": "~*", "exclude": true }
{ "pattern": "openrouter/auto", "exclude": true }
```

BuckyOS owns its own soft-link system through `logical_mounts`.

## Variants

Variants describe provider options that must be part of the AICC exact model
identity instead of ordinary request parameters. They currently apply to LLM
models.

```json
{
  "name": "reasoning.high",
  "mount_suffix": "reasoning-high",
  "provider_options": {
    "reasoning": {
      "effort": "high"
    }
  }
}
```

For a discovered OpenAI model `gpt-5.1`, the resolver emits:

- base exact model: `gpt-5.1@openai-primary`
- variant exact model: `gpt-5.1:reasoning-high@openai-primary`

Route output for the variant uses `provider_model_id = "gpt-5.1"` and returns
the variant `provider_options`. Provider adapters receive the same lowered base
model and options even when callers invoke the variant exact model directly.

## Version Rules

Version rules add derived mounts after the base model rule is resolved. They do
not replace `models`, `patterns`, or `defaults`; they operate on the matched
model family and tier.

```json
{
  "family": "gpt",
  "tier": "standard",
  "model_pattern": "gpt-*",
  "exclude_tier_tokens": ["pro", "mini", "nano"],
  "version_rank": { "prefix": "gpt" },
  "stability": {
    "unstable_tokens": ["preview", "experimental", "beta"],
    "current_requires_stable": true
  },
  "current_mount": "llm.gpt-standard",
  "version_mount": "llm.openai.{model}",
  "auto_mounts": ["llm", "llm.plan", "llm.code"],
  "exclude_snapshot_date_suffix": true
}
```

## Cloud Authoring And Materialization

The cloud authoring model contains source references, provider keys, and origin
identity authoring rules. They are administrative metadata and are not emitted
as storage records in this final driver metadata document. `name`,
`provider_driver`, `protocol_family`, `origin_provider_aliases`, and
`origin_mappings` are final client-facing fields and must be emitted together.

- `provider_driver` identifies the driver metadata document and is the value of
  its `provider_driver` field. Cloud delivery filenames use this id.
- `protocol_family` identifies the client-to-provider wire protocol. It is
  emitted in this document so clients can select the correct protocol adapter.
  Multiple providers can share one protocol family.
- Cloud authoring stores exact, pattern, and default rules uniformly, but
  materialization always emits exact rules in `models`, ordered pattern rules
  in `patterns`, and at most one default in `defaults`. Pattern authoring uses
  explicit list order; materialization writes the corresponding stable
  priority values.
- Cloud nick/origin identity rules can construct provider-native selectors from
  source metadata, for example turning an OpenAI source pattern `gpt-*` into an
  OpenRouter provider pattern `openai/gpt-*`. This selector construction is
  separate from `origin_mappings`: final `models[].id`, `patterns[].pattern`,
  variant selectors, and version-rule `model_pattern` must be provider-native
  ids, while `origin_mappings` defines how those provider-native ids resolve
  back to `{driver}` and `{model}` for logical mount expansion.
- Version-rule authoring must preserve the complete predicate, including
  family, tier, `model_pattern`, excluded tokens, stability and version-rank
  conditions. `model_pattern` alone is not a complete description of its
  match specificity.
- Cloud authoring may use a materialized directory tree to choose mount paths.
  In the Add Provider wizard, the dedicated Logical mounts step writes
  model/pattern/default `logical_mounts` and version-rule `auto_mounts`.
  Variant `logical_mounts` remains part of the driver metadata schema, but the
  Add Provider Logical mounts step preserves variant mount values from the
  source or draft instead of editing them.
  Version-rule `current_mount` and `version_mount` are separate single-value
  mount fields selected from the same materialized directory tree and emitted
  as mount strings.
