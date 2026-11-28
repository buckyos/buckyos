import { modelChoices, modelFits, needsAttention, summarizeCoverage, targetOfView, usecaseImpact, type UsecaseRow } from '../../src/app/ai-center/datamodel/usecases.ts'
import type { ModelMetadata } from '../../src/app/ai-center/mock/types.ts'

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}

function row(usecase: string, model: string | undefined, coverage: UsecaseRow['coverage'], extra: Partial<UsecaseRow> = {}): UsecaseRow {
  return { usecase, api_type: 'llm', model, on_unavailable: 'none', on_unavailable_source: 'default', coverage, ...extra }
}

function model(exact: string, driver: string, apiTypes: string[], capabilities: Partial<ModelMetadata['capabilities']> = {}): ModelMetadata {
  return {
    provider_model_id: exact.split('@')[0],
    exact_model: exact,
    model_driver: driver,
    api_types: apiTypes as ModelMetadata['api_types'],
    logical_mounts: [],
    capabilities: { streaming: true, tool_call: false, json_schema: false, web_search: false, vision: false, ...capabilities },
    attributes: { local: false, privacy: 'public_cloud', latency_class: 'normal', cost_class: 'medium' },
    pricing: {},
    health: { status: 'available', quota_state: 'normal' },
  }
}

Deno.test('coverage splits newly covered, changed and still uncovered usecases', () => {
  const summary = summarizeCoverage([
    { usecase: 'llm.chat', before_coverage: 'uncovered', after: 'a@x', after_coverage: 'covered' },
    { usecase: 'llm.plan', before: 'a@x', before_coverage: 'covered', after: 'b@y', after_coverage: 'covered' },
    { usecase: 'llm.code', before: 'a@x', before_coverage: 'covered', after: 'a@x', after_coverage: 'covered' },
    { usecase: 'llm.vision', before: 'f@x', before_coverage: 'fallback_llm', after: 'f@x', after_coverage: 'fallback_llm' },
    { usecase: 'llm.swift', before: 'f@x', before_coverage: 'fallback_llm', after: 'c@y', after_coverage: 'covered' },
  ])
  assert(summary.newlyCovered.map((change) => change.usecase).join() === 'llm.chat,llm.swift', 'fallback-served usecases that get a model are newly covered')
  assert(summary.changed.map((change) => change.usecase).join() === 'llm.plan', 'only a different covered model is a change')
  assert(summary.stillUncovered.map((change) => change.usecase).join() === 'llm.vision', 'fallback LLM is not coverage')
  assert(summary.unchanged === 1, 'unchanged counted')
})

Deno.test('impact lists usecases whose model or coverage changes without a Provider', () => {
  const impact = usecaseImpact(
    [row('llm.chat', 'a@x', 'covered'), row('llm.code', 'b@y', 'covered'), row('video.generate', 'v@x', 'covered'), row('llm.fallback', 'f@x', 'covered')],
    [row('llm.chat', 'c@z', 'covered'), row('llm.code', 'b@y', 'covered'), row('video.generate', undefined, 'uncovered'), row('llm.fallback', undefined, 'uncovered')],
  )
  assert(impact.length === 2, 'unchanged rows and the fallback LLM are left out')
  assert(impact[0].usecase === 'llm.chat' && impact[0].after === 'c@z', 'who takes over')
  assert(impact[1].usecase === 'video.generate' && impact[1].coverage === 'uncovered', 'becomes uncovered')
})

Deno.test('model choices fit the API type and requirements, grouped by family', () => {
  const models = [
    model('gpt@openai', 'openai', ['llm'], { tool_call: true, max_context_tokens: 200000 }),
    model('mini@openai', 'openai', ['llm'], { max_context_tokens: 8000 }),
    model('sonnet@claude', 'claude', ['llm'], { tool_call: true, max_context_tokens: 200000 }),
    model('embed@openai', 'openai', ['embedding.text']),
  ]
  assert(!modelFits(models[1], 'llm', { tool_call: true }), 'missing tool calls')
  assert(!modelFits(models[0], 'llm', { min_context_tokens: 300000 }), 'context too short')
  const families = modelChoices(models, 'llm', { tool_call: true })
  assert(families.map((family) => family.family).join() === 'claude,openai', 'families sorted')
  assert(families[1].models.map((entry) => entry.exact_model).join() === 'gpt@openai', 'only fitting models')
})

Deno.test('views write their own layer; attention covers rows without their own model and skipped layers', () => {
  assert(targetOfView('self') === 'user' && targetOfView('zone') === 'zone' && targetOfView('group:g1') === 'group:g1', 'targets')
  assert(targetOfView('app:aiworkspace') === null, 'an App view is read-only')
  assert(needsAttention(row('llm.vision', 'f@x', 'fallback_llm')), 'served by the fallback LLM')
  assert(needsAttention(row('llm.chat', 'a@x', 'covered', { layers: [{ layer: 'user', model: 'gone@x', model_state: 'missing' }] })), 'a skipped layer model')
  assert(!needsAttention(row('llm.chat', 'a@x', 'covered')), 'covered')
  assert(!needsAttention(row('llm.fallback', undefined, 'uncovered')), 'the fallback LLM row itself')
})
