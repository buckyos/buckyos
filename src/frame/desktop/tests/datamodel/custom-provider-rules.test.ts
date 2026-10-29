import {
  applyModelRuleEdits, createCustomRulesDraft, disabledRuleModels, setRuleModelEnabled, modelRuleDraft, paginateRuleModels,
  parseProviderRulesFile, providerRulesFile, previewRules, ruleObject, rulesJson, toProviderRuleOptions, ruleModelDefinition, replaceScopedRequestRules, validateRuleEditor,
} from '../../src/api/custom_provider_rules.ts'

function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message)
}

Deno.test('model edits round-trip all categories and preserve unrelated rules', () => {
  const base = createCustomRulesDraft({ instance_rules: { policy_region: 'unknown' } })
  const resolved = { local: { rule: { request_rules: [{ remove: ['/temperature'] }] } } }
  const changed = applyModelRuleEdits(base, { local: {
    alias: 'openai/gpt-6-astra', routes: 'llm.chat, llm.reason, llm.chat',
    operations: '{"llm":"chat.completions"}', variants: '[]',
    pricing: '{"currency":"USD","input_token":0.1,"output_token":0.2}',
  } }, resolved)
  const parsed = parseProviderRulesFile(rulesJson(providerRulesFile(changed)), base)
  const row = modelRuleDraft(parsed, 'local')
  assert(row.alias === 'openai/gpt-6-astra', 'alias must survive')
  assert(row.routes === 'llm.chat, llm.reason', 'tasks must be explicit and deduplicated')
  assert(row.variants === '[]', 'empty variants must override inherited variants')
  assert(JSON.parse(row.pricing).input_token === 0.1, 'price must survive')
  const exact = (ruleObject(parsed.instance_rules.catalog).models as Record<string, unknown>[])[0]
  assert(JSON.stringify(exact.request_rules) === '[{"remove":["/temperature"]}]', 'editing operations must preserve request rules')
  assert(parsed.instance_rules.policy_region === 'unknown', 'unrelated instance rules must remain')
  assert((previewRules(parsed, 'pricing') as unknown[]).length === 1, 'price preview must contain the edit')
  assert(JSON.stringify(ruleObject(base.instance_rules.catalog).models) === '[]', 'original document must remain unchanged')
})

Deno.test('deleting aliases removes inherited mappings from the instance catalog', () => {
  const file = createCustomRulesDraft({ provider_rules_id: 'selected-preset', rules_catalog: {
    ...ruleObject(createCustomRulesDraft({}).instance_rules.catalog),
    model_driver_overrides: { local: 'openai/gpt-6-astra' },
  } })
  const changed = applyModelRuleEdits(file, { local: { alias: '' } }, {})
  assert(modelRuleDraft(changed, 'local').alias === '', 'deleted aliases must not reappear')
})

Deno.test('import rejects secrets, malformed versions and invalid task lists', () => {
  const cases = [
    (file: Record<string, unknown>) => { file.api_key = 'secret' },
    (file: Record<string, unknown>) => { file.format = 'buckyos.aicc.custom-provider-rules' },
    (file: Record<string, unknown>) => { file.schema_version = 99 },
    (file: Record<string, unknown>) => { file.revision_seq = Number.MAX_SAFE_INTEGER + 1 },
    (file: Record<string, unknown>) => { file.credentials = {} },
    (file: Record<string, unknown>) => { file.server_default_routes = { local: ['image.txt2img'] } },
    (file: Record<string, unknown>) => { file.models = {} },
  ]
  for (const change of cases) {
    const file = providerRulesFile(createCustomRulesDraft({}))
    change(file)
    let rejected = false
    try { parseProviderRulesFile(rulesJson(file)) } catch { rejected = true }
    assert(rejected, 'invalid imports must be rejected before mutation')
  }
})

Deno.test('pagination orders enabled models first, searches case-insensitively and clamps pages', () => {
  const result = paginateRuleModels(['z-model', 'A-model', 'b-model', 'A-model'], ['A-model'], 'MODEL', 0, 2)
  assert(JSON.stringify(result.models) === '["b-model","z-model"]', 'enabled models must come first')
  assert(result.total === 3 && result.pages === 2, 'duplicate IDs must not become duplicate rows')
  assert(paginateRuleModels(['x'], [], 'missing', 10, 20).page === 0, 'empty searches must clamp the page')
})

Deno.test('pagination scales without per-model RPC calls', () => {
  for (const size of [1, 10, 1000, 10000]) {
    const ids = Array.from({ length: size }, (_, i) => `model-${String(i).padStart(5, '0')}`)
    const start = performance.now()
    const result = paginateRuleModels(ids, ids.slice(0, 3), '', 69, 20)
    assert(result.models.length <= 20 && result.total === size, 'only one page is rendered')
    console.log(`${size} models: ${(performance.now() - start).toFixed(2)} ms; no RPC calls`)
  }
})

Deno.test('a saved instance catalog takes precedence over an unavailable runtime preset', () => {
  const saved = ruleObject(createCustomRulesDraft({}).instance_rules.catalog)
  saved.model_variants = { local: [] }
  const file = createCustomRulesDraft({
    instance_rules: { catalog: saved },
    rules_catalog: { ...saved, model_variants: {} },
  })
  assert(modelRuleDraft(file, 'local').variants === '[]', 'disabled or unloaded instances must retain their saved rules')
})


Deno.test('enabling a model overrides inherited exclusions without discarding its protocol rules', () => {
  const file = createCustomRulesDraft({})
  const resolved = { local: { rule: { exclude: true, operations: { llm: 'responses.create' } } } }
  assert(disabledRuleModels(file, resolved).includes('local'), 'inherited exclusions must be shown')
  const enabled = setRuleModelEnabled(file, 'local', true, resolved)
  assert(!disabledRuleModels(enabled, resolved).includes('local'), 'explicit enable must override inherited exclusions')
  assert(modelRuleDraft(enabled, 'local').operations.includes('responses.create'), 'protocol mappings must survive enabling')
  const disabled = setRuleModelEnabled(enabled, 'local', false, resolved)
  assert(disabledRuleModels(disabled, resolved).includes('local'), 'explicit disable must survive')
})

Deno.test('custom rules are populated from the instance inventory without unbound preset prices', () => {
  const builtin = ruleObject(createCustomRulesDraft({}).instance_rules.catalog)
  builtin.models = [{ id: 'unrelated-gemini', operations: { llm: 'interactions.create' } }]
  builtin.model_pricing = [{ id: 'gemini-4-argon', pricing: { currency: 'USD', input_token: 0.000002 } }]
  const actual = {
    'gemini-4-argon': {
      operations: { llm: 'interactions.background' },
      variants: [{ model_driver: 'gemini', variant: 'reasoning-low', match: '*', provider_options: { generation_config: { thinking_level: 'low' } } }],
      pricing: null,
    },
    'priced-by-instance': {
      operations: { llm: 'interactions.create' },
      variants: [],
      pricing: { currency: 'USD', input_token: 0.0000003 },
    },
  }
  const file = createCustomRulesDraft({ rules_catalog: builtin, resolved_model_rules: actual })
  const row = modelRuleDraft(file, 'gemini-4-argon', actual['gemini-4-argon'])
  assert(row.operations === '{}', 'adapter defaults must not become explicit operation mappings')
  assert(row.pricing === 'null', 'missing instance prices must remain empty')
  assert(JSON.parse(modelRuleDraft(file, 'priced-by-instance', actual['priced-by-instance']).pricing).input_token === 0.0000003, 'the instance discovery price must remain visible')
  const prices = previewRules(file, 'pricing') as Record<string, unknown>[]
  assert(prices.length === 0, 'unconfigured prices must not be written merely by previewing')
  const models = ruleObject(file.instance_rules.catalog).models as Record<string, unknown>[]
  assert(models.length === 0, 'unbound built-in models and automatic mappings must not enter the configuration')
  const saved = parseProviderRulesFile(rulesJson(providerRulesFile(applyModelRuleEdits(file, {}, actual))), file)
  assert(modelRuleDraft(saved, 'gemini-4-argon').pricing === 'null', 'a no-op save must not add built-in prices')
})

Deno.test('an explicitly selected preset remains an instance rule source', () => {
  const preset = ruleObject(createCustomRulesDraft({}).instance_rules.catalog)
  preset.model_pricing = [{ id: 'configured-model', pricing: { currency: 'USD', input_token: 0.0000002 } }]
  const file = createCustomRulesDraft({ provider_rules_id: 'chosen-provider', rules_catalog: preset })
  assert(JSON.parse(modelRuleDraft(file, 'configured-model').pricing).input_token === 0.0000002, 'explicitly bound rules must remain editable')
})

function editorOptions() {
  return toProviderRuleOptions({
    revision: 1, vendors: [{ id: 'gemini', revision: 1, specs: [], models: [{
      id: 'local-model', providers: [], metadata: {
        api_types: ['llm', 'image.txt2img'],
        llm: { spec: 'gemini', effort: 'native', default_effort: 'native', supported_efforts: ['native', 'low', 'high'], stability: 'stable' },
      },
    }] }],
  }, [
    { path: 'llm', kind: 'task', api_type: 'llm' },
    { path: 'llm.chat', kind: 'task', api_type: 'llm' },
    { path: 'llm.spec', kind: 'spec', api_type: 'llm' },
    { path: 'llm.family', kind: 'family', api_type: 'llm' },
    { path: 'image.generate', kind: 'task', api_type: 'image.txt2img' },
  ], [{
    protocol_adapter_id: 'gemini-interactions',
    default_operations: { llm: 'interactions.background' },
    operations: [{ operation_id: 'interactions.create', api_types: ['llm'] }, { operation_id: 'interactions.background', api_types: ['llm'] }],
  }])
}

Deno.test('structured editor choices use model metadata, finite LLM tasks and adapter bindings', () => {
  const options = editorOptions()
  assert(JSON.stringify(options.tasks) === '["llm.chat"]', 'only LLM tasks are valid choices')
  assert(options.models[0].efforts.join(',') === 'native,low,high', 'efforts must come from model metadata')
  assert(options.adapters[0].defaults.llm === 'interactions.background', 'adapter defaults must come from the server')
  const file = createCustomRulesDraft({})
  const row = modelRuleDraft(file, 'endpoint')
  row.alias = 'gemini/local-model'
  assert(ruleModelDefinition('endpoint', row, {}, options)?.apiTypes.includes('llm'), 'the selected built-in model must drive API choices')
  const edits = { endpoint: { alias: row.alias, routes: 'llm.chat', operations: '{}', variants: '[]' } }
  const saved = applyModelRuleEdits(file, edits, {})
  validateRuleEditor(saved, edits, {}, options, 'gemini-interactions')
  assert(modelRuleDraft(saved, 'endpoint').operations === '{}', 'using a default must not create an operation override')
  for (const invalid of [
    { ...edits.endpoint, routes: 'llm.spec' },
    { ...edits.endpoint, operations: '{"llm":"images.generate"}' },
    { ...edits.endpoint, variants: '[{"model_driver":"gemini","variant":"reasoning-max","match":"*","provider_options":{}}]' },
  ]) {
    let rejected = false
    try { const data = { endpoint: invalid }; validateRuleEditor(applyModelRuleEdits(file, data, {}), data, {}, options, 'gemini-interactions') } catch { rejected = true }
    assert(rejected, 'unsupported tasks, operations and efforts must be rejected')
  }
})

Deno.test('per-API request rule editing preserves global rules and other API scopes', () => {
  const source = rulesJson([
    { remove: ['/temperature'] },
    { when: { api_type: ['llm', 'image.txt2img'], operation: 'interactions.create' }, set: { shared: true } },
    { when: { api_type: 'audio.tts' }, defaults: { voice: 'a' } },
  ])
  const updated = JSON.parse(replaceScopedRequestRules(source, 'llm', [{ set: { stream: true } }]))
  assert(updated[0].remove[0] === '/temperature', 'global rules must remain')
  assert(updated[1].when.api_type.join(',') === 'image.txt2img', 'other scopes of a shared rule must remain')
  assert(updated[2].when.api_type === 'audio.tts', 'unrelated API rules must remain')
  assert(updated[3].when.api_type[0] === 'llm' && updated[3].set.stream, 'updated rules must be scoped to the selected API')
})

Deno.test('effort configuration distinguishes an explicit empty object from an unsupported effort', () => {
  const options = editorOptions()
  const file = createCustomRulesDraft({})
  const edits = { endpoint: {
    alias: 'gemini/local-model',
    variants: '[{"model_driver":"gemini","variant":"reasoning-low","match":{"provider_model_id":"endpoint"},"provider_options":{}}]',
    effort_options: { low: '{}' },
    scoped_request_rules: { llm: '[{"remove":["/temperature"]}]' },
  } }
  const changed = applyModelRuleEdits(file, edits, {})
  validateRuleEditor(changed, edits, {}, options, 'gemini-interactions')
  const variants = JSON.parse(modelRuleDraft(changed, 'endpoint').variants)
  assert(variants.length === 1 && Object.keys(variants[0].provider_options).length === 0, 'an explicit empty provider_options mapping must remain enabled')
  const disabled = applyModelRuleEdits(changed, { endpoint: { variants: '[]', effort_options: { low: '{' } } }, {})
  assert(modelRuleDraft(disabled, 'endpoint').variants === '[]', 'disabled effort drafts must not be saved or cause JSON errors')
})

Deno.test('provider JSON export uses the existing catalog schema and round-trips exclusions and tasks', () => {
  const base = createCustomRulesDraft({ provider_rules_id: 'chosen', instance_rules: { policy_region: 'unknown', exclude_models: ['hidden'] } })
  const edited = applyModelRuleEdits(base, { local: { alias: 'claude/claude-opus-5-5', routes: 'llm.chat' } }, {})
  const file = providerRulesFile(edited)
  assert(file.format === 'buckyos.aicc.provider-rules-catalog', 'must use the .provider.json format')
  assert(!('instance_rules' in file) && !('provider_rules_id' in file) && !('policy_region' in file), 'instance settings must stay outside the file')
  assert(ruleObject(file.server_default_routes).local instanceof Array, 'tasks must be part of the provider catalog')
  const imported = parseProviderRulesFile(rulesJson(file), base)
  assert(imported.instance_rules.policy_region === 'unknown' && imported.provider_rules_id === 'chosen', 'import must retain instance settings')
  assert(disabledRuleModels(imported, {}).includes('hidden'), 'excluded models must survive as model rules')
  assert(modelRuleDraft(imported, 'local').routes === 'llm.chat', 'task lists must survive')
  assert(JSON.stringify(providerRulesFile(imported)) === JSON.stringify(file), 'export/import must preserve the catalog')
  const fromCatalog = createCustomRulesDraft({ instance_rules: { catalog: file, server_default_routes: {} } })
  assert(modelRuleDraft(fromCatalog, 'local').routes === 'llm.chat', 'catalog tasks must populate the editor without an instance override')
  const cleared = applyModelRuleEdits(fromCatalog, { local: { routes: '' } }, {})
  assert(!('local' in ruleObject(providerRulesFile(cleared).server_default_routes)), 'clearing a task list must remove catalog mappings')
})

Deno.test('existing builtin provider JSON files can be imported without an envelope', async () => {
  const directory = new URL('../../../aicc/driver_metadata/providers/', import.meta.url)
  let count = 0
  for await (const entry of Deno.readDir(directory)) {
    if (!entry.name.endsWith('.provider.json')) continue
    const source = await Deno.readTextFile(new URL(entry.name, directory))
    const draft = parseProviderRulesFile(source)
    const original = JSON.parse(source)
    const roundTrip = providerRulesFile(draft)
    assert(roundTrip.provider_profile_id === original.provider_profile_id, entry.name)
    for (const [key, value] of Object.entries(original)) {
      assert(JSON.stringify(roundTrip[key]) === JSON.stringify(value), `${entry.name}: ${key} changed its value or structure`)
    }
    assert(!('server_default_routes' in roundTrip) || 'server_default_routes' in original, 'absent task settings must not add a new field')
    assert(JSON.stringify(providerRulesFile(draft).variants ?? []) === JSON.stringify(original.variants ?? []), `${entry.name}: variants must be preserved without renaming`)
    assert(!('model_variants' in providerRulesFile(draft)), 'provider files must use variants only')
    count++
  }
  assert(count > 0, 'must verify real provider files')
})

Deno.test('per-model effort edits export standard variants without changing other model scopes', () => {
  const base = createCustomRulesDraft({})
  const catalog = ruleObject(base.instance_rules.catalog)
  catalog.variants = [
    { model_driver: 'openai', variant: 'reasoning-low', match: { origin_model_id: ['gpt-6-sol', 'gpt-6-luna'], variant: 'reasoning-low' }, provider_options: { reasoning: { effort: 'low' } } },
    { model_driver: 'openai', variant: 'reasoning-high', match: { provider_model_id: ['first', 'second'], variant: 'reasoning-high' }, provider_options: { reasoning: { effort: 'high' } } },
  ]
  const changed = applyModelRuleEdits(base, { first: { variants: '[{"model_driver":"openai","variant":"reasoning-low","match":{"provider_model_id":"first"},"provider_options":{}}]' } }, {})
  const file = providerRulesFile(changed)
  assert(!('model_variants' in file), 'export must not contain the invented field')
  const variants = file.variants as Record<string, unknown>[]
  assert(JSON.stringify(ruleObject(variants[0].match).provider_model_id) === '{"not":["first"]}', 'shared rules must exclude only the edited endpoint')
  assert(JSON.stringify(ruleObject(variants[1].match).provider_model_id) === '["second"]', 'other endpoint bindings must remain')
  assert(ruleObject(variants[2].match).provider_model_id === 'first', 'edited effort must be scoped by the existing match syntax')
  const imported = parseProviderRulesFile(rulesJson(file))
  assert(JSON.parse(modelRuleDraft(imported, 'first').variants).length === 1, 'standard variants must round-trip')
  const disabled = providerRulesFile(applyModelRuleEdits(imported, { first: { variants: '[]' } }, {}))
  assert((disabled.variants as Record<string, unknown>[]).length === 2, 'removing an effort must not leave a matching duplicate')
  const invalid = { ...file, model_variants: {} }
  let rejected = false
  try { parseProviderRulesFile(rulesJson(invalid)) } catch { rejected = true }
  assert(rejected, 'the file importer must reject the nonstandard field')
})
