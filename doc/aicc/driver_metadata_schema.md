# AICC Driver Metadata Schema

Driver metadata turns provider-discovered model ids into AICC `ModelMetadata`.
Provider `/models` discovery is only the id source; the resolver owns capability,
API type, mount, cost, latency, and conservative fallback decisions.

## Source Priority

The resolver loads metadata in this override order:

1. builtin metadata bundled under `src/frame/aicc/driver_metadata/`
2. latest complete cloud activation under `$BUCKYOS_ROOT/data/srv/aicc/driver_metadata/remote_cache/v1/<source-key>/`
3. local override: `$BUCKYOS_ROOT/etc/aicc/driver_metadata/local/<driver>.json`
4. system-config override materialized at `$BUCKYOS_ROOT/etc/aicc/driver_metadata/system-config/<driver>.json`

For each model id, match priority is:

1. exact `models[].id`
2. wildcard `patterns[].pattern`
3. `defaults`
4. conservative fallback

Exact matches win before patterns, even if the pattern comes from a higher
priority override.

`origin_mappings` is a provider-level rule set, not an incremental patch. The
resolver uses the complete list from the highest-priority metadata document
that defines it and does not merge mappings from lower-priority documents.
Therefore, an override document that defines `origin_mappings` must include the
provider's complete mapping list.

## Document

```json
{
  "format": "buckyos.aicc.provider-driver-metadata",
  "schema_version": 3,
  "schema_revision": 0,
  "provider_driver": "openai",
  "revision_seq": 1,
  "required_features": [],
  "origin_provider_aliases": {},
  "origin_mappings": [],
  "models": [],
  "patterns": [],
  "defaults": {},
  "variants": [],
  "signature": null
}
```

Fields:

 - `format`: fixed to `buckyos.aicc.provider-driver-metadata`.
 - `schema_version`: incompatible schema major, currently `3`.
 - `schema_revision`: additive schema revision, currently `0`.
 - `provider_driver`: driver id such as `openai`, `openrouter`, `claude`,
  `google-gemini`, `fal`, or `minimax`.
 - `revision_seq`: monotonically increasing unsigned integer for this provider.
 - `required_features`: features that a reader must understand; unknown values reject the document.
 - `origin_provider_aliases`: optional map from provider-specific origin slugs to
  canonical BuckyOS driver names.
 - `origin_mappings`: optional ordered rules that derive the physical origin
  `driver` and `model` from the provider-native model id.
- `models`: exact rules keyed by `id`.
- `patterns`: wildcard rules keyed by `pattern`; `*` is the only wildcard.
- `defaults`: default rule when no exact or pattern rule matches.
- `variants`: optional provider option variants. The resolver expands each
  matching base model into additional AICC exact models whose provider model id
  is `<base>:<mount_suffix>`, while provider calls are lowered back to the base
  provider model plus `provider_options`.
- `signature`: deprecated compatibility field. Cloud trust comes from the NDN PathObject and FileObject chain.

The beta 2.2 schema is the first compatibility baseline. Future optional fields
with a safe default increment `schema_revision`; incompatible changes increment
`schema_version` and are published on a compatible protocol track.

Cloud and local override documents use the same strict parser. Unknown fields,
unsupported features, schema/provider identity mismatches, and statically invalid
rules are rejected before they can participate in inventory construction.

## Rule

Rules support these fields:

- `id`: exact provider model id for `models`.
- `pattern`: wildcard provider model id pattern for `patterns`.
- `model_driver`: optional per-model metadata driver. Defaults to the resolved
  origin driver. A rule can override it when metadata attribution differs from
  the physical model origin.
- `exclude`: drops the provider model from inventory.
- `parameter_scale`: optional display/classification string.
- `api_types`: AICC API types, for example `llm.chat`, `image.txt2img`, `audio.asr`.
- `logical_mounts`: logical mounts. Templates `{driver}`, `{model}`,
  `{provider_driver}`, and `{provider_model_id}` are expanded by the resolver.
  The first pair identifies the physical origin; the second pair identifies the
  current delivery channel.
- `capabilities`: partial capability patch: `streaming`, `tool_call`, `json_schema`, `web_search`, `web_search_with_tool_call`, `vision`, `max_context_tokens`, `max_output_tokens`. `web_search_with_tool_call` is only needed when a model's support for combining provider-hosted search with custom function calling differs from its individual `web_search` and `tool_call` capabilities; when omitted it defaults to the conjunction of those individual capabilities.
- `pricing`: optional monetary data object. `currency` identifies the ISO 4217
  currency; `input_token`, `output_token`, and `cache_input_token` are optional
  per-token prices; `estimated_cost` is the default scheduler cost estimate.
- `estimated_latency_ms`: default scheduler latency estimate.
- `quality_score`, `latency_class`, `cost_class`: routing attributes.

All exact ids and wildcard patterns, including
`version_rules[].model_pattern`, match the complete channel-local
`provider_model_id`. Origin fields are only used for metadata attribution and
mount template expansion. For example, OpenAI uses `gpt-*`, while OpenRouter
uses `openai/gpt-*` for the same origin model family.

Unknown fallback is intentionally conservative: it does not declare
`tool_call`, `web_search`, `vision`, or `json_schema`.

## Origin Identity Mappings

Provider model ids are channel-local. An origin provider such as OpenAI can use
the fallback identity `openai` / `gpt-5.5`. An aggregator such as OpenRouter
returns a channel id such as `openai/gpt-5.5`, which must resolve to the same
physical identity before logical mounts and semantic family rules are applied.
The resolver stores the resolved model component in
`ModelMetadata.origin_model_id`; consumers must use this field instead of
inferring an origin model from provider-specific id syntax.

Schema v2 defines these template variables:

| Variable | Meaning |
| --- | --- |
| `{driver}` | resolved physical origin driver, for example `openai` |
| `{model}` | resolved physical origin model, for example `gpt-5.5` |
| `{provider_driver}` | current channel driver, for example `openrouter` |
| `{provider_model_id}` | current channel model id, for example `openai/gpt-5.5` |

Mappings are evaluated by ascending `priority`; the first successful rule wins.
If no rule succeeds, the resolver uses `provider_driver` and
`provider_model_id` as the origin identity.

```json
{
  "origin_provider_aliases": {
    "google": "google-gemini",
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
          { "op": "lowercase" },
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

The regex must use named `driver` and `model` captures. Supported transforms
are `trim`, `lowercase`, and `alias`; alias lookup only accepts the
`origin_provider_aliases` table and `on_missing: keep`. Unknown fields, invalid
regular expressions, missing named captures, unsupported sources or transforms,
and invalid transform options reject the complete metadata document before
activation. Dynamic provider aliases such as `~x-ai/grok-latest` and router models
such as `openrouter/auto` should be excluded with ordinary model or pattern
rules. Aggregators that only admit base OpenAI model ids should place
`openai/*:*` and `openai/*latest*` exclusion patterns before family allow
patterns so provider variants and moving aliases cannot inherit base-model
metadata.

## Variants

Variants describe provider options that must be part of the AICC exact model
identity instead of ordinary request parameters. They currently apply to LLM
models.

```json
{
  "name": "reasoning.high",
  "model_pattern": "gpt-*",
  "mount_suffix": "reasoning-high",
  "provider_options": {
    "reasoning": {
      "effort": "high"
    }
  }
}
```

`model_pattern` matches the complete channel-local `provider_model_id`. It can
scope variants by origin within an aggregator, for example `openai/*` in
OpenRouter metadata. When omitted, the variant applies to every otherwise
eligible model in that driver metadata document.

For a discovered OpenAI model `gpt-5.1`, the resolver emits:

- base exact model: `gpt-5.1@openai-primary`
- variant exact model: `gpt-5.1:reasoning-high@openai-primary`

Route output for the variant uses `provider_model_id = "gpt-5.1"` and returns
the variant `provider_options`. Provider adapters receive the same lowered base
model and options even when callers invoke the variant exact model directly.
