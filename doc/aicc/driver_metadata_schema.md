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
  "revision": "builtin-2026-05-30",
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

- `schema_version`: currently `2`.
- `provider_driver`: driver id such as `openai`, `openrouter`, `claude`,
  `google-gemini`, `fal`, or `minimax`.
- `revision`: monotonically changing metadata revision string.
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
- `signature`: optional signature envelope; verification is not enforced yet.

## Rule

Rules support these fields:

- `id`: exact provider model id for `models`.
- `pattern`: wildcard provider model id pattern for `patterns`.
- `model_driver`: optional per-model metadata driver. Defaults to the resolved
  origin driver. A rule can override it when metadata attribution differs from
  the physical model origin.
- `provider_actual_model_id`: optional model id sent to the provider instead of
  the discovered id. Aggregators can use it to pin a public alias to the
  inventory's canonical fixed model revision.
- `exclude`: drops the provider model from inventory.
- `parameter_scale`: optional display/classification string.
- `api_types`: AICC API types, for example `llm.chat`, `image.txt2img`, `audio.asr`.
- `logical_mounts`: logical mounts. Templates `{driver}`, `{model}`,
  `{provider_driver}`, and `{provider_model_id}` are expanded by the resolver.
  The first pair identifies the physical origin; the second pair identifies the
  current delivery channel.
- `capabilities`: partial capability patch: `streaming`, `tool_call`, `json_schema`, `web_search`, `vision`, `max_context_tokens`, `max_output_tokens`.
- `input_token_usd`, `output_token_usd`, `cache_input_token_usd`: optional
  provider token prices in USD per token.
- `estimated_cost_usd`, `estimated_latency_ms`: default scheduler estimates.
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
`origin_provider_aliases` table and `on_missing: keep`. Invalid mappings are
ignored. Dynamic provider aliases such as `~x-ai/grok-latest` and router models
such as `openrouter/auto` should be excluded with ordinary model or pattern
rules.

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
