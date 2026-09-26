import { expect, test, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { defaultRequest, fieldDefault, importRequest, PLAYGROUND_APIS, record, resourceUrl, taskResponse, validateRequest, type JsonObject, type PlaygroundField } from '../../src/app/ai-center/datamodel/playground'
import type { ApiType } from '../../src/app/ai-center/mock/types'

const apis = Object.keys(PLAYGROUND_APIS) as ApiType[]
const resource = { kind: 'url', url: 'https://example.com/input.png' }
function fillField(field: PlaygroundField): unknown {
  if (field.kind === 'resource') return resource
  if (field.kind === 'text') return 'sample'
  if (field.kind === 'object') return Object.fromEntries(field.fields!.filter((child) => child.required).map((child) => [child.key, fillField(child)]))
  if (field.kind === 'array') return [fillField(field.item!)]
  if (field.kind === 'union') {
    const variant = Object.keys(field.variants!)[0]
    return { type: variant, ...Object.fromEntries(field.variants![variant].filter((child) => child.required).map((child) => [child.key, fillField(child)])) }
  }
  if (field.kind === 'checks') return [field.options![0]]
  return fieldDefault(field)
}
function sample(api: ApiType): JsonObject {
  const params = { ...defaultRequest(api), exact_model: `${api}@test-provider`, ...Object.fromEntries(PLAYGROUND_APIS[api].fields.filter((field) => field.required).map((field) => [field.key, fillField(field)])) }
  if (api === 'embedding.multimodal') params.items = [{ id: 'item-1', text: 'hello' }]
  if (api === 'rerank') params.documents = [{ id: 'doc-1', text: 'hello' }]
  return params
}

async function serviceFixture(page: Page, mode = 'success') {
  await page.route('**/src/api/aicc_playground.ts*', (route) => route.fulfill({ contentType: 'application/javascript', body: `
    import { mockPlaygroundResponse } from '/src/app/ai-center/mock/playground.ts';
    let polls = 0;
    let listAttempts = 0;
    let cancelled = false;
    export async function listPlaygroundModels() {
      if (${JSON.stringify(mode)} === 'loadError' && listAttempts++ === 0) throw new Error('offline');
      if (${JSON.stringify(mode)} === 'empty') return [];
      return ${JSON.stringify(apis.map((api) => ({ exact_model: `${api}@test-provider`, provider: 'test-provider', api_types: [api] })))};
    }
    export async function invokePlayground(api, params) {
      globalThis.playgroundLastRequest = {api, params};
      if (${JSON.stringify(mode)} === 'rpcError') throw Object.assign(new Error('provider timeout'), {code: 'timeout'});
      if (${JSON.stringify(mode)} === 'invalid') return {status: 'running', detail: 'missing task_id'};
      if (['async', 'cancel', 'pollError', 'failed'].includes(${JSON.stringify(mode)})) return {status: 'running', task_id: 'task-42', provider_task_ref: 'provider-42'};
      return mockPlaygroundResponse(api, params);
    }
    export async function getPlaygroundTask(taskId) {
      if (${JSON.stringify(mode)} === 'pollError' && polls++ === 0) throw new Error('temporarily offline');
      if (${JSON.stringify(mode)} === 'cancel' && !cancelled || ${JSON.stringify(mode)} === 'async' && polls++ === 0) return {task_id: taskId, phase: 'Running', progress: {events: [{type: 'delta', text: 'partial output'}]}};
      if (cancelled) return {task_id: taskId, phase: 'Terminal', outcome: 'Canceled'};
      if (${JSON.stringify(mode)} === 'failed') return {task_id: taskId, phase: 'Terminal', outcome: 'Failed', error: {code: 'provider_error', message: 'Unsupported model', detail: {provider_code: '404'}}};
      return {task_id: taskId, phase: 'Terminal', outcome: 'Succeeded', result: {result: {output: {value: {message: {role: 'assistant', content: [{type: 'text', text: 'Async completed'}]}, finish_reason: 'stop'}, usage: {total_tokens: 42}, cost: {amount: 0.02, currency: 'USD'}, artifacts: []}}}};
    }
    export async function cancelPlaygroundTask(taskId) {cancelled = true; return {task_id: taskId, accepted: true};}
  ` }))
}
async function openPlayground(page: Page, chinese = false) {
  await page.addInitScript((locale) => localStorage.setItem('buckyos.prototype.locale.v1', locale), chinese ? 'zh-CN' : 'en')
  await page.goto('/?aiccScenario=populated')
  if ((page.viewportSize()?.width ?? 1280) < 768) await page.getByTestId('desktop-app-ai-center').tap()
  else await page.getByTestId('desktop-app-ai-center').click()
  await page.getByRole('button', { name: 'Playground', exact: true }).click()
}
async function llmInput(page: Page) {
  await page.getByRole('combobox', { name: 'Exact model', exact: true }).selectOption('llm@test-provider')
  await page.getByRole('textbox', { name: 'messages[0].content[0].text', exact: true }).fill('Hello')
}

test('canonical API coverage, task request wrappers, identifier cleanup and resource validation', () => {
  const rust = readFileSync('../../kernel/buckyos-api/src/aicc_client.rs', 'utf8')
  const aiMethods = rust.slice(rust.indexOf('pub fn is_ai_method'), rust.indexOf('pub fn is_aicc_core_method'))
  const methods = [...rust.matchAll(/pub const (\w+): &str = "([^"]+)";/g)].filter((match) => new RegExp(`\\b${match[1]}\\b`).test(aiMethods)).map((match) => match[2])
  expect(Object.values(PLAYGROUND_APIS).map((api) => api.method).sort()).toEqual(methods.sort())
  for (const api of apis) {
    const params = sample(api)
    expect(validateRequest(api, params), api).toEqual([])
    const fullTask = { name: `AICC ${PLAYGROUND_APIS[api].method}`, input: { request: { version: 1, request: { ...params, idempotency_key: 'old', trace_id: 'old-trace', task_options: { parent_id: 'old-parent' }, session_id: 'keep-session', extra: { retained: true } } } } }
    const imported = importRequest(JSON.stringify(fullTask), 'llm')
    expect(imported.api).toBe(api)
    expect(imported.params).toEqual({ ...params, session_id: 'keep-session', extra: { retained: true } })
    expect(imported.removed).toEqual(['idempotency_key', 'trace_id', 'task_options'])
    expect(importRequest(JSON.stringify({ method: PLAYGROUND_APIS[api].method, params }), 'llm').params).toEqual(params)
    expect(importRequest(JSON.stringify(params), api).api).toBe(api)
  }
  expect(validateRequest('llm', { ...sample('llm'), messages: [{ role: 'user', content: [{ type: '__proto__' }] }] })).toContainEqual({ path: 'messages[0].content[0]', code: 'invalid' })
  expect(() => importRequest('{broken', 'llm')).toThrow()
  expect(() => importRequest(JSON.stringify({ method: 'provider.delete', params: sample('llm') }), 'llm')).toThrow('unsupportedApi')
  expect(validateRequest('image.inpaint', { ...sample('image.inpaint'), mask: { kind: 'url', url: 'javascript:alert(1)' } })).toContainEqual({ path: 'mask', code: 'resource' })
  expect(validateRequest('audio.asr', { ...sample('audio.asr'), audio: { kind: 'base64', mime: 'audio/wav', data_base64: 'broken!' } })).toContainEqual({ path: 'audio', code: 'resource' })
  expect(validateRequest('llm', { ...sample('llm'), temperature: 3, max_output_tokens: -1 })).toHaveLength(2)
  expect(validateRequest('decision', { ...sample('decision'), execution_mode: 'stream' })).toContainEqual({ path: 'execution_mode', code: 'invalid' })
  const mixedDecision = { exact_model: 'decision@test-provider', state: { context: ['structured state'] }, questions: [
    { type: 'choice', id: 'choice-1', instructions: { rule: 'Choose' }, options: [{ id: 'yes', description: null }, { id: 'no', description: ['No'] }] },
    { type: 'score', id: 'score-1', instructions: 'Rate', levels: [{ description: 'Low' }, ['High']] },
    { type: 'boolean', id: 'bool-1', instructions: 'Check', criteria: { true: { condition: 'yes' }, false: ['no'] } },
  ] }
  expect(validateRequest('decision', mixedDecision)).toEqual([])
  expect(importRequest(JSON.stringify({ method: 'decision.evaluate', params: mixedDecision }), 'llm').params).toEqual(mixedDecision)
  expect(validateRequest('decision', { ...mixedDecision, questions: [mixedDecision.questions[0], mixedDecision.questions[0]] })).toContainEqual({ path: 'questions', code: 'invalid' })
  expect(resourceUrl({ kind: 'url', url: 'javascript:alert(1)' })).toBeUndefined()
  expect(resourceUrl({ kind: 'base64', mime: 'text/html', data_base64: 'PGgxPng8L2gxPg==' })).toBeUndefined()
  expect(resourceUrl({ kind: 'base64', mime: 'image/png', data_base64: 'aGk=' })).toBe('data:image/png;base64,aGk=')
  const result = taskResponse({ task_id: 'id', phase: 'Terminal', outcome: 'Succeeded', result: { result: { output: { value: { text: 'done' }, usage: { total_tokens: 8 }, cost: { amount: 0.1, currency: 'USD' } } } } }, { status: 'running', route_trace: { final_model: 'm@p' } })
  expect(result).toMatchObject({ status: 'succeeded', text: 'done', usage: { total_tokens: 8 }, cost: { amount: 0.1 }, route_trace: { final_model: 'm@p' } })
  expect(taskResponse({ phase: 'Terminal', outcome: 'Canceled' }, {}).status).toBe('cancelled')
})

test('all API forms import and submit canonical parameters without losing extensions', async ({ page }) => {
  test.setTimeout(90000)
  const errors: string[] = []
  page.on('pageerror', (error) => errors.push(error.message))
  await serviceFixture(page)
  await openPlayground(page)
  await page.getByRole('checkbox', { name: 'Advanced mode / import request JSON' }).check()
  for (const api of apis) {
    const params = { ...sample(api), session_id: 'session-keep' }
    await page.getByRole('textbox', { name: 'Request JSON to import' }).fill(JSON.stringify({ method: PLAYGROUND_APIS[api].method, params }))
    await page.getByRole('button', { name: 'Extract parameters', exact: true }).click()
    await expect(page.getByRole('combobox', { name: 'API Type', exact: true })).toHaveValue(api)
    await page.getByRole('button', { name: 'Run request', exact: true }).click()
    await expect(page.getByRole('region', { name: 'Call result' })).toContainText('Succeeded')
    const submitted = await page.evaluate(() => (globalThis as unknown as { playgroundLastRequest: unknown }).playgroundLastRequest)
    expect(submitted).toEqual({ api, params })
  }
  expect(errors).toEqual([])
})

test('default mock runtime uses connected provider inventory', async ({ page }) => {
  await openPlayground(page)
  const model = page.getByRole('combobox', { name: 'Exact model', exact: true })
  await expect(model.locator('option')).not.toHaveCount(1)
  await model.selectOption({ index: 1 })
  await page.getByRole('textbox', { name: 'messages[0].content[0].text', exact: true }).fill('Hello')
  await page.getByRole('button', { name: 'Run request', exact: true }).click()
  await expect(page.getByRole('region', { name: 'Call result' })).toContainText('Succeeded')
})

test('guided chat, validation, navigation persistence and full result details', async ({ page }, testInfo) => {
  await serviceFixture(page)
  await openPlayground(page)
  await page.getByRole('combobox', { name: 'Exact model', exact: true }).selectOption('llm@test-provider')
  await page.getByRole('button', { name: 'Run request', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('messages[0].content[0].text')
  await llmInput(page)
  await page.getByRole('checkbox', { name: 'Enable temperature', exact: true }).check()
  await page.getByRole('spinbutton', { name: 'temperature', exact: true }).fill('0.5')
  await page.getByRole('button', { name: 'Run request', exact: true }).click()
  await expect(page.getByRole('region', { name: 'Call result' })).toContainText('Mock response')
  await expect(page.getByRole('region', { name: 'Call result' })).toContainText('total_tokens')
  await page.getByRole('button', { name: 'Models', exact: true }).click()
  await page.getByRole('button', { name: 'Playground', exact: true }).click()
  await expect(page.getByRole('textbox', { name: 'messages[0].content[0].text', exact: true })).toHaveValue('Hello')
  await page.getByText('Full response JSON', { exact: true }).click()
  await expect(page.locator('details').filter({ hasText: 'Full response JSON' })).toContainText('route_trace')
  await page.getByRole('heading', { name: 'Playground', exact: true }).scrollIntoViewIfNeeded()
  await page.screenshot({ path: testInfo.outputPath('playground-desktop.png') })
})

test('URL, data URL and file input, plus invalid import leaves the form intact', async ({ page }) => {
  await serviceFixture(page)
  await openPlayground(page)
  await page.getByRole('combobox', { name: 'API Type', exact: true }).selectOption('image.inpaint')
  await page.getByRole('combobox', { name: 'Exact model', exact: true }).selectOption('image.inpaint@test-provider')
  await page.getByRole('textbox', { name: 'image URL', exact: true }).fill('https://example.com/photo.png')
  await page.getByRole('combobox', { name: 'mask Input mode', exact: true }).selectOption('base64')
  await page.getByRole('textbox', { name: 'mask Base64', exact: true }).fill('data:image/png;base64,aGk=')
  await page.getByRole('textbox', { name: 'prompt', exact: true }).fill('Change the background')
  await page.getByRole('button', { name: 'Run request', exact: true }).click()
  await expect(page.getByRole('region', { name: 'Call result' })).toContainText('Succeeded')
  await page.getByRole('combobox', { name: 'image Input mode', exact: true }).selectOption('file')
  await page.getByLabel('image Choose file', { exact: true }).setInputFiles({ name: 'image.png', mimeType: 'image/png', buffer: Buffer.from('image bytes') })
  await expect(page.getByText('Reading file…')).toBeHidden()
  await page.getByRole('button', { name: 'Run request', exact: true }).click()
  await expect(page.getByRole('region', { name: 'Call result' })).toContainText('Succeeded')
  const submitted = await page.evaluate(() => (globalThis as unknown as { playgroundLastRequest: { params: JsonObject } }).playgroundLastRequest.params)
  expect(submitted.image).toEqual({ kind: 'base64', mime: 'image/png', data_base64: Buffer.from('image bytes').toString('base64') })
  expect(submitted.mask).toEqual({ kind: 'base64', mime: 'image/png', data_base64: 'aGk=' })
  await page.getByRole('checkbox', { name: 'Advanced mode / import request JSON' }).check()
  await page.getByRole('textbox', { name: 'Request JSON to import' }).fill('{bad')
  await page.getByRole('button', { name: 'Extract parameters', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('Enter valid JSON')
  await expect(page.getByRole('textbox', { name: 'prompt', exact: true })).toHaveValue('Change the background')
})

for (const mode of ['async', 'pollError', 'cancel', 'failed', 'rpcError', 'invalid']) {
  test(`call lifecycle: ${mode}`, async ({ page }) => {
    await serviceFixture(page, mode)
    await openPlayground(page)
    await llmInput(page)
    await page.getByRole('button', { name: 'Run request', exact: true }).click()
    if (mode === 'pollError') {
      await expect(page.getByRole('alert')).toContainText('does not mean the Provider call failed')
      await page.getByRole('button', { name: 'Resume task monitoring', exact: true }).click()
    }
    if (mode === 'cancel') {
      await page.getByRole('button', { name: 'Cancel task', exact: true }).click()
      await expect(page.getByRole('region', { name: 'Call result' })).toContainText('Cancelled')
    } else if (mode === 'failed') {
      await expect(page.getByRole('region', { name: 'Call result' })).toContainText('Failed')
      await expect(page.getByRole('alert')).toContainText('provider_code')
    } else if (mode === 'rpcError') await expect(page.getByRole('alert')).toContainText('provider timeout')
    else if (mode === 'invalid') await expect(page.getByRole('alert')).toContainText('unexpected response')
    else {
      await expect(page.getByRole('region', { name: 'Call result' })).toContainText('Async completed')
      await expect(page.getByRole('region', { name: 'Call result' })).toContainText('42')
      await expect(page.getByRole('link', { name: 'Open in Task Center: task-42', exact: true })).toHaveAttribute('href', '/taskcenter?taskid=task-42')
    }
    await expect(page.getByRole('button', { name: 'Run request', exact: true })).toBeEnabled()
  })
}

test('empty models and model-load retry', async ({ page }) => {
  await serviceFixture(page, 'loadError')
  await openPlayground(page)
  await expect(page.getByRole('alert')).toContainText('Could not load models')
  await page.getByRole('button', { name: 'Retry', exact: true }).click()
  await expect(page.getByRole('combobox', { name: 'Exact model', exact: true }).locator('option')).toHaveCount(2)
  await page.unroute('**/src/api/aicc_playground.ts*')
  await serviceFixture(page, 'empty')
  await page.reload()
  await page.getByTestId('desktop-app-ai-center').click()
  await page.getByRole('button', { name: 'Playground', exact: true }).click()
  await expect(page.getByText('No connected model supports', { exact: false })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Run request', exact: true })).toBeDisabled()
})

test.describe('touch viewport', () => {
  test.use({ hasTouch: true })

test('mobile Chinese page fits the viewport', async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 375, height: 812 })
  await serviceFixture(page)
  await openPlayground(page, true)
  await page.getByRole('combobox', { name: '精确模型', exact: true }).selectOption('llm@test-provider')
  await page.getByRole('textbox', { name: 'messages[0].content[0].text', exact: true }).fill('你好')
  await page.getByRole('button', { name: '发起调用', exact: true }).click()
  await expect(page.getByRole('region', { name: '调用结果' })).toContainText('调用成功')
  const main = page.locator('main').filter({ has: page.getByRole('heading', { name: 'Playground', exact: true }) }).last()
  expect(await main.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true)
  await page.screenshot({ path: testInfo.outputPath('playground-mobile.png') })
})

})

test('Task Center retains original AICC input alongside a terminal result', async ({ page }) => {
  await page.goto('/?aiccScenario=populated')
  const mapped = await page.evaluate(async () => {
    const path = '/src/api/task_mgr.ts'
    const { toTaskCenterTask } = await import(path)
    return toTaskCenterTask({ summary: { task_id: 'finished', root_id: 'finished', creator: {app_id: 'desktop', user_id: 'alice'}, schema_id: 'aicc.compute/v1', name: 'AICC chat.completions.create', phase: 'Terminal', outcome: 'Succeeded', created_at: 100, updated_at: 200 }, detail: { input: { request: { request: { exact_model: 'm@p', messages: [{ role: 'user', content: [{ type: 'text', text: 'Original input' }] }] } } }, result: { result: { output: { value: { text: 'Completed answer' } } } } } })
  })
  expect(mapped.aiccRequest.method).toBe('chat.completions.create')
  expect(mapped.aiccRequest.params.messages[0].content[0].text).toBe('Original input')
  expect(record(mapped.payload)).toHaveProperty('result')
})

test('file completion preserves recent edits and cannot overwrite another API form', async ({ page }) => {
  await serviceFixture(page)
  await openPlayground(page)
  await page.evaluate(() => {
    const original = FileReader.prototype.readAsDataURL
    FileReader.prototype.readAsDataURL = function (blob) {
      const resumeFileRead = original.bind(this, blob)
      ;(globalThis as unknown as { resumeFileRead: () => void }).resumeFileRead = resumeFileRead
    }
  })
  await page.getByRole('combobox', { name: 'API Type', exact: true }).selectOption('image.inpaint')
  await page.getByRole('combobox', { name: 'image Input mode', exact: true }).selectOption('file')
  const file = { name: 'image.png', mimeType: 'image/png', buffer: Buffer.from('image') }
  await page.getByLabel('image Choose file', { exact: true }).setInputFiles(file)
  await expect(page.getByText('Reading file…')).toBeVisible()
  await page.getByRole('textbox', { name: 'prompt', exact: true }).fill('Latest prompt')
  await page.evaluate(() => (globalThis as unknown as { resumeFileRead: () => void }).resumeFileRead())
  await expect(page.getByText('Reading file…')).toBeHidden()
  await expect(page.getByRole('textbox', { name: 'prompt', exact: true })).toHaveValue('Latest prompt')
  await page.getByLabel('image Choose file', { exact: true }).setInputFiles({ ...file, name: 'next.png' })
  await expect(page.getByText('Reading file…')).toBeVisible()
  await page.getByRole('combobox', { name: 'API Type', exact: true }).selectOption('audio.tts')
  await page.getByRole('textbox', { name: 'text', exact: true }).fill('Keep this text')
  await page.evaluate(() => (globalThis as unknown as { resumeFileRead: () => void }).resumeFileRead())
  await page.getByText('Request preview', { exact: true }).click()
  await expect(page.getByRole('textbox', { name: 'text', exact: true })).toHaveValue('Keep this text')
  await expect(page.locator('details').filter({ hasText: 'Request preview' })).not.toContainText('data_base64')
})
