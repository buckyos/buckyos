import { z } from 'zod'

export interface CatalogModel {
  id: string
  metadata: {
    api_types?: string[]
    parameter_scale?: string
    local_deployable?: boolean
    capabilities?: Record<string, unknown>
    llm?: {
      spec: string
      effort: string
      default_effort: string
      supported_efforts: string[]
      stability: string
    }
    [key: string]: unknown
  }
  providers: { id: string; local: boolean; exact_models: string[] }[]
}

export interface CatalogSpec {
  id: string
  path: string
  direct_only: boolean
  members: { model_id: string; target: string; weight: number; active: boolean }[]
}

export interface ModelCatalog {
  revision: number
  vendors: { id: string; revision: number; models: CatalogModel[]; specs: CatalogSpec[] }[]
}

export interface ModelCardView extends CatalogModel {
  vendorId: string
  available: boolean
  local: boolean
  deployable: boolean
}

export const modelFiltersSchema = z.object({
  query: z.string(),
  available: z.boolean(),
  deployable: z.boolean(),
  local: z.boolean(),
})

export type ModelFilters = z.infer<typeof modelFiltersSchema>
export const defaultModelFilters: ModelFilters = { query: '', available: false, deployable: false, local: false }

export const vendorNames: Record<string, string> = {
  openai: 'OpenAI', claude: 'Anthropic', anthropic: 'Anthropic', gemini: 'Google Gemini',
  qwen: 'Qwen', deepseek: 'DeepSeek', glm: 'Z.ai GLM', kimi: 'Moonshot Kimi',
  typesafe: 'TypeSafe', minimax: 'MiniMax', doubao: 'Doubao', cohere: 'Cohere', fal: 'fal',
}

export function modelCard(model: CatalogModel, vendorId: string): ModelCardView {
  return {
    ...model,
    vendorId,
    available: model.providers.length > 0,
    local: model.providers.some((provider) => provider.local),
    deployable: model.metadata.local_deployable === true,
  }
}

export function filterModelCatalog(catalog: ModelCatalog, filters: ModelFilters) {
  const query = filters.query.trim().toLocaleLowerCase()
  const hasStatusFilter = filters.available || filters.deployable || filters.local
  return catalog.vendors.map((vendor) => {
    const vendorMatches = `${vendor.id} ${vendorNames[vendor.id] ?? ''}`.toLocaleLowerCase().includes(query)
    const models = vendor.models.map((model) => modelCard(model, vendor.id)).filter((model) => {
      const matches = vendorMatches || [model.id, model.metadata.llm?.spec, ...(model.metadata.api_types ?? [])]
        .some((value) => value?.toLocaleLowerCase().includes(query))
      return matches && (!filters.available || model.available) && (!filters.local || model.local)
        && (!filters.deployable || model.deployable)
    }).sort((a, b) => a.id.localeCompare(b.id, undefined, { numeric: true }))
    const visible = new Set(models.map((model) => model.id))
    const specs = vendor.specs.map((spec) => ({
      ...spec, members: spec.members.filter((member) => visible.has(member.model_id)),
    })).filter((spec) => spec.members.length > 0 || (!hasStatusFilter && (vendorMatches || spec.id.toLocaleLowerCase().includes(query))))
    return { ...vendor, models, specs }
  }).filter((vendor) => vendor.models.length || vendor.specs.length)
}
