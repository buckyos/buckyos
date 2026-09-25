# AI Center UI DataModel

## Overview

WP-17A uses `/kapi/aicc` as its only backend. Provider setup is driven by `provider.catalog` and `protocol_adapter.list`; Provider instances, models, routing, usage, and traces use their corresponding AICC management methods. AI Center does not read or write system-config or control-panel AICC helpers.

## Provider setup

```ts
interface ProviderSetupCatalog {
  catalog_revision: number
  providers: KnownProviderProfile[]
  protocol_families: ProtocolFamilyOption[]
}

interface WizardDraft {
  provider_instance_name?: string
  provider_profile_id: string | null
  display_name: string
  base_url: string
  operation_base_urls: Record<string, string>
  protocol_family_id: string | null
  protocol_adapter_id?: string
  region?: string
  workspace?: string
  account?: string
  policy_region?: string
  auth_mode: 'api_key' | 'dynamic_login'
  api_key: string
  auto_sync_models: boolean
}
```

`provider_profile_id`, `protocol_adapter_id`, `base_url`, and `operation_base_urls` are contract fields. `operation_base_urls` maps an operation such as `tts.unidirectional` to the endpoint used instead of the provider's default `base_url`. `ui_hints` is extensible. `display_name` is UI-only. For built-in profiles the adapter, default URLs, and optional/required region, workspace, and account fields are selected by the catalog. The UI presents `base_url` as an editable combobox backed by catalog `region_base_urls`: its independent dropdown always shows every configured endpoint regardless of the current text; selecting one updates both `base_url` and `region`, while a custom URL sets `region` to `unknown`. Each `operation_base_urls` value uses the same editable-combobox behavior and at minimum lists its configured default so it can be restored after manual editing. The catalog region `default_value` is the broadest applicable endpoint and is selected initially; even a single configured endpoint remains available as a reset choice. `policy_region` is the separate residence/account-policy dimension and never changes a URL. Custom providers expose an editable, required `base_url` and only `protocol_family_id`; the resolved adapter is read back from validation.

`provider_profile_id` is an open catalog identity. The UI preserves IDs introduced after the desktop build and does not maintain a Provider allowlist. Only Protocol Adapter implementations remain client-version capabilities.

Catalog loading has distinct loading, error/retry, empty, and success states. One Wizard open issues one catalog request and one adapter-registry request; it does not perform per-profile reads.

## Provider instances and conflicts

`provider.list` supplies every enabled and disabled instance plus `settings_revision`. Provider updates and routing updates use this revision for CAS. `settings_revision_conflict` causes the store to reload the latest snapshot before the UI asks the user to retry. Credentials are write-only and represented in the UI only by `credential_configured` and `auth_mode`.

## Usage finance

```ts
interface Money {
  amount: number
  currency: string
}

interface UsageSummary {
  total_tokens: number
  total_requests: number
  finance_totals: Money[]
  finance_complete: boolean
}
```

Currencies are never converted or summed together. Codes are normalized to uppercase, duplicate currency rows are merged, and totals are ordered by descending numeric amount. A single currency is shown directly. Multiple currencies initially show the largest amount and expose a click target that expands or collapses the full list. Incomplete finance aggregation is preserved and labeled as partial.

The transform is O(n + c log c), where `n` is the returned row count and `c` is the number of currencies; memory is O(c). It has no additional RPC reads. Raw usage events retain their own finance snapshot currency.

## KRPC mapping

| UI field | Method | Backend field |
| --- | --- | --- |
| Known provider profiles | `provider.catalog` | `providers[]` |
| Custom protocol families | `protocol_adapter.list` | unique `adapters[custom_provider_selectable=true].protocol_family_id` |
| Provider instances | `provider.list` | `providers[]` |
| Provider revision | `provider.list` | `settings_revision` |
| Provider credential/status update | `provider.update` | typed update request |
| Routing weights | `routing.get` / `routing.update` | `provider_weights` |
| Models | `models.list` | `models[]` |
| Usage totals | `usage.query` | `total.finance_totals[]` |
| Route traces | `trace.query` | `traces[]` |
