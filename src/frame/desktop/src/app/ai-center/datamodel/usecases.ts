// The usecase table, its layers, routing groups and App bindings (`routing.usecases`, `routing.usecase.set`,
// `routing.groups.*`, `routing.bindings.*`); the wire shapes are `buckyos-api/src/aicc_routing.rs`.
import type { ModelMetadata } from '../mock/types'

export type OnUnavailable = 'none' | 'same_model' | 'not_pricier' | 'next_model'
export const ON_UNAVAILABLE: OnUnavailable[] = ['none', 'same_model', 'not_pricier', 'next_model']

export type UsecaseCoverage = 'covered' | 'fallback_llm' | 'uncovered'
export type UsecaseModelState = 'available' | 'missing' | 'not_permitted' | 'unsuitable'
export type UsecaseField = 'model' | 'on_unavailable'

/** The fallback LLM: off by default; an administrator turns it on so LLM usecases without a model still work. */
export const FALLBACK_LLM = 'llm.fallback'

export const USECASES = [
  'decision',
  'llm.chat', 'llm.plan', 'llm.code', 'llm.swift', 'llm.summarize', 'llm.translate', 'llm.vision',
  'embedding.text', 'embedding.multimodal', 'rerank',
  'image.txt2img', 'image.img2img', 'image.inpaint', 'image.upscale', 'image.bg_remove',
  'vision.ocr', 'vision.caption', 'vision.detect', 'vision.segment',
  'audio.tts', 'audio.asr', 'audio.music', 'audio.enhance',
  'video.generate', 'video.edit', 'video.extend', 'video.upscale',
  'agent_runtime.computer_use',
] as const

/** The LLM usecases the fallback LLM serves when they have no model. */
export const LLM_USECASES = ['llm.chat', 'llm.plan', 'llm.code', 'llm.swift', 'llm.summarize', 'llm.translate', 'llm.vision']

export interface LockedValue<T> { value: T; locked?: boolean }

export interface UsecaseRoute {
  model?: LockedValue<string>
  on_unavailable?: LockedValue<OnUnavailable>
  /** Only `llm.fallback`, only the administrator layer. */
  enabled?: boolean
}

export type UsecaseRoutes = Record<string, UsecaseRoute>

export interface UsecaseLayerValue {
  /** `zone`, `user`, `group:<id>`. */
  layer: string
  model?: string
  model_locked?: boolean
  model_state?: UsecaseModelState
  on_unavailable?: OnUnavailable
  on_unavailable_locked?: boolean
  enabled?: boolean
}

export interface UsecaseRow {
  usecase: string
  api_type: string
  model?: string
  /** `default`, `zone`, `user`, `group:<id>` or `fallback_llm`. */
  model_source?: string
  on_unavailable: OnUnavailable
  on_unavailable_source: string
  model_locked?: boolean
  on_unavailable_locked?: boolean
  layers?: UsecaseLayerValue[]
  /** What the policy would use if the Provider of `model` were unavailable. */
  fallback_to?: string
  /** Why `fallback_to` is empty under a policy other than `none`. */
  fallback_note?: string
  coverage: UsecaseCoverage
  /** `no_model_for_api_type` or `requirements_unmet` when uncovered. */
  coverage_reason?: string
  /** The routing error, for administrators. */
  detail?: string
  /** The `llm.fallback` row: whether the fallback LLM is on. */
  enabled?: boolean
}

/** `self`, `zone` (administrators), `group:<id>`, `app:<app_id>`. */
export type UsecaseView = string

export interface RoutingUsecasesResponse {
  view: UsecaseView
  /** What `routing.usecase.set` of this view's layer passes. */
  revision: number
  settings_revision: number
  fallback_llm_enabled: boolean
  can_edit: boolean
  rows: UsecaseRow[]
}

export interface UsecaseSetRequest {
  /** `zone`, `user` or `group:<id>`. */
  target: string
  usecase: string
  model?: string | LockedValue<string>
  on_unavailable?: OnUnavailable | LockedValue<OnUnavailable>
  enabled?: boolean
  clear?: UsecaseField[]
  revision: number
}

export interface RoutingGroup {
  group_id: string
  name: string
  usecases?: UsecaseRoutes
  version: number
  updated_at_ms: number
  app_ids?: string[]
}

export interface RoutingBinding { app_id: string; group_id: string }

/** One usecase before and after a Provider is added (`provider.validate`). */
export interface UsecaseCoverageChange {
  usecase: string
  before?: string
  before_coverage: UsecaseCoverage
  after?: string
  after_coverage: UsecaseCoverage
}

/** Hard requirements of a usecase (`min_line` of its logical definition) that the model picker checks. */
export interface UsecaseRequirement {
  streaming?: boolean
  tool_call?: boolean
  json_schema?: boolean
  web_search?: boolean
  vision?: boolean
  min_context_tokens?: number
}

/** An App or Agent a routing group can be bound to (an Agent's app id is its agent id). */
export interface RoutingApp { app_id: string; label: string; agent: boolean }

/** The layer a view writes: `zone`, `user`, `group:<id>`; an App view is read-only. */
export function targetOfView(view: UsecaseView): string | null {
  if (view === 'zone') return 'zone'
  if (view === 'self') return 'user'
  if (view.startsWith('group:')) return view
  return null
}

/** The value the view's own layer holds for a row, if any. */
export function ownLayer(row: UsecaseRow, view: UsecaseView): UsecaseLayerValue | undefined {
  const target = targetOfView(view)
  return target ? row.layers?.find((layer) => layer.layer === target) : undefined
}

/** A layer model that is set but skipped (missing, hidden or unsuitable), so the row follows a lower layer. */
export function skippedLayers(row: UsecaseRow): UsecaseLayerValue[] {
  return (row.layers ?? []).filter((layer) => layer.model && layer.model_state && layer.model_state !== 'available')
}

/** A row needs the viewer's attention: not covered by its own model, or a layer's choice is skipped. */
export function needsAttention(row: UsecaseRow): boolean {
  if (row.usecase === FALLBACK_LLM) return false
  return row.coverage !== 'covered' || skippedLayers(row).length > 0
}

export interface CoverageSummary {
  newlyCovered: UsecaseCoverageChange[]
  changed: UsecaseCoverageChange[]
  stillUncovered: UsecaseCoverageChange[]
  unchanged: number
}

/** What adding a Provider does to each usecase: newly covered, a different default model, still without one. */
export function summarizeCoverage(changes: UsecaseCoverageChange[]): CoverageSummary {
  const summary: CoverageSummary = { newlyCovered: [], changed: [], stillUncovered: [], unchanged: 0 }
  for (const change of changes) {
    if (change.after_coverage !== 'covered') summary.stillUncovered.push(change)
    else if (change.before_coverage !== 'covered') summary.newlyCovered.push(change)
    else if (change.before && change.after && change.before !== change.after) summary.changed.push(change)
    else summary.unchanged += 1
  }
  return summary
}

export interface UsecaseImpact {
  usecase: string
  before?: string
  after?: string
  coverage: UsecaseCoverage
}

/** Usecases whose model or coverage differs between two tables (a Provider deleted or disabled). */
export function usecaseImpact(current: UsecaseRow[], without: UsecaseRow[]): UsecaseImpact[] {
  const next = new Map(without.map((row) => [row.usecase, row]))
  return current.flatMap((row) => {
    const after = next.get(row.usecase)
    if (!after || row.usecase === FALLBACK_LLM) return []
    if (after.model === row.model && after.coverage === row.coverage) return []
    return [{ usecase: row.usecase, before: row.model, after: after.model, coverage: after.coverage }]
  })
}

/** Whether a model can serve a usecase: its API type and the usecase's hard requirements. */
export function modelFits(model: ModelMetadata, apiType: string, requirement: UsecaseRequirement = {}): boolean {
  if (!(model.api_types as string[]).includes(apiType)) return false
  const capabilities = model.capabilities
  if (requirement.streaming && !capabilities.streaming) return false
  if (requirement.tool_call && !capabilities.tool_call) return false
  if (requirement.json_schema && !capabilities.json_schema) return false
  if (requirement.web_search && !capabilities.web_search) return false
  if (requirement.vision && !capabilities.vision) return false
  if (requirement.min_context_tokens && (capabilities.max_context_tokens ?? 0) < requirement.min_context_tokens) return false
  return true
}

export interface ModelFamily { family: string; models: ModelMetadata[] }

/** The models a usecase can choose, grouped by model family (Model Driver), each family sorted by name. */
export function modelChoices(models: ModelMetadata[], apiType: string, requirement?: UsecaseRequirement): ModelFamily[] {
  const families = new Map<string, ModelMetadata[]>()
  const seen = new Set<string>()
  for (const model of models) {
    if (seen.has(model.exact_model) || !modelFits(model, apiType, requirement)) continue
    seen.add(model.exact_model)
    families.set(model.model_driver, [...(families.get(model.model_driver) ?? []), model])
  }
  return [...families.entries()]
    .map(([family, entries]) => ({ family, models: entries.sort((left, right) => left.exact_model.localeCompare(right.exact_model, undefined, { numeric: true })) }))
    .sort((left, right) => left.family.localeCompare(right.family))
}

/** `min_line` of a logical definition as the picker checks it. */
export function toUsecaseRequirement(value: unknown): UsecaseRequirement {
  if (!value || typeof value !== 'object') return {}
  const raw = value as Record<string, unknown>
  const flag = (key: string) => (raw[key] === true ? { [key]: true } : {})
  return {
    ...flag('streaming'),
    ...flag('tool_call'),
    ...flag('json_schema'),
    ...flag('web_search'),
    ...flag('vision'),
    ...(typeof raw.min_context_tokens === 'number' ? { min_context_tokens: raw.min_context_tokens } : {}),
  }
}
