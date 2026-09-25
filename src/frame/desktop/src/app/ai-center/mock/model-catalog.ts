import type { CatalogModel, ModelCatalog } from '../datamodel/model-catalog'
import type { StoreSnapshot } from './types'

const definitions = [
  { vendor: 'openai', id: 'gpt-5.1', spec: 'gpt-standard', context: 256000 },
  { vendor: 'openai', id: 'gpt-5.1-mini', spec: 'gpt-mini', context: 128000 },
  { vendor: 'openai', id: 'gpt-5.6', spec: 'gpt-standard', context: 1000000 },
  { vendor: 'claude', id: 'claude-sonnet-4.5', spec: 'sonnet', context: 200000 },
  { vendor: 'claude', id: 'claude-opus-4.7', spec: 'opus', context: 1000000 },
  { vendor: 'qwen', id: 'qwen2.5-coder-32b', spec: 'qwen-coder', context: 128000, local: true },
  { vendor: 'qwen', id: 'qwen3.5-27b', spec: 'qwen-dense-27b', context: 262144, local: true },
]

export function mockModelCatalog(snapshot: StoreSnapshot): ModelCatalog {
  const vendors = [...new Set(definitions.map((model) => model.vendor))].map((id) => {
    const models: CatalogModel[] = definitions.filter((model) => model.vendor === id).map((definition) => {
      const providers = snapshot.providers.filter((provider) => provider.config.enabled).flatMap((provider) => {
        const instances = provider.inventory.models.filter((model) => model.provider_model_id === definition.id)
        return instances.length ? [{ id: provider.config.id, local: provider.config.provider_runtime_type === 'local_inference', exact_models: instances.map((model) => model.exact_model) }] : []
      })
      const local = snapshot.localModels.filter((model) => model.provider_model_id === definition.id)
      if (local.length) providers.push({ id: 'local', local: true, exact_models: local.map((model) => model.exact_model) })
      return {
        id: definition.id,
        metadata: {
          api_types: ['llm'], local_deployable: definition.local ?? false,
          capabilities: { streaming: true, tool_call: true, max_context_tokens: definition.context },
          llm: { spec: definition.spec, effort: 'native', default_effort: 'native', supported_efforts: ['native'], stability: 'stable' },
        },
        providers,
      }
    })
    const specs = [...new Set(models.map((model) => model.metadata.llm!.spec))].map((spec) => ({
      id: spec, path: `llm.${spec}`, direct_only: false,
      members: models.filter((model) => model.metadata.llm!.spec === spec).map((model) => ({
        model_id: model.id, target: `llm.${model.id.replaceAll('.', '-')}:native`, weight: 1, active: model.providers.length > 0,
      })),
    }))
    if (id === 'qwen') specs.push({ id: 'qwen-code', path: 'llm.qwen-code', direct_only: true, members: [] })
    return { id, revision: 1, models, specs }
  })
  return { revision: 1, vendors }
}
