import { readFileSync } from 'node:fs'
import { expect, test } from '@playwright/test'

test('image inventory hints and unsupported parameter errors survive browser SDK handling', async ({ page }) => {
  await page.route('**/__image_contract_test', route => route.fulfill({ contentType: 'text/html', body: '<html></html>' }))
  await page.goto('/__image_contract_test')
  const result = await page.evaluate(async () => {
    const { imageModelCapabilities, classified } = await import('/src/app/aiworkspace/executor/infer.ts')
    const { providerErrorFromRpc, providerError } = await import('/node_modules/buckyos/dist/xllm.mjs')
    const text = await imageModelCapabilities(async () => ({ models: Array.from({ length: 40 }, (_, n) => ({
      exact_model: `image-${n}@provider`, secret: 'private-value', capabilities: { 'image.img2img.aspect_ratio': true, 'image.img2img.quality': false },
    })) }))
    const error = classified(providerErrorFromRpc('image', { code: 'unsupported_operation', message: 'negative_prompt for image@test' }))
    const unknown = classified(providerError({ refused: { code: 'future_refusal' } }, 'request refused'))
    return { text, code: error.code, unknownCode: unknown.code }
  })
  expect(result.text).toContain('image-31@provider')
  expect(result.text).not.toContain('image-32@provider')
  expect(result.text).not.toContain('private-value')
  expect(result.text).not.toContain('image.img2img.quality')
  expect(result.code).toBe('AI_UNSUPPORTED_OPERATION')
  expect(result.unknownCode).toBe('AI_UNAVAILABLE')
})

for (const mode of ['failed-response', 'failed-task']) {
  test(`media ${mode} keeps its code through the sandbox macro and WASM state`, async ({ page }) => {
    const hostSource = readFileSync(new URL('../../../aiworkspace/aiws/host.js', import.meta.url), 'utf8')
    await page.route('**/__image_contract_host.js', route => route.fulfill({ contentType: 'text/javascript', body: hostSource }))
    await page.route('**/__image_contract_test', route => route.fulfill({ contentType: 'text/html', body: '<html></html>' }))
    await page.goto('/__image_contract_test')
    const result = await page.evaluate(async mode => {
      const { createBridgeHost } = await import('/__image_contract_host.js')
      const { runProgram } = await import('/src/app/aiworkspace/executor/programHost.ts')
      const { default: init, wish_main_status } = await import('/src/app/aiworkspace/wasm/aiworkspace_wasm.js')
      await init()
      const error = { code: 'unsupported_operation', message: 'quality is unsupported for selected-model@test-provider' }
      const host = createBridgeHost({
        tool: async () => ({}), infer: async () => ({}), importAsset: async () => { throw new Error('unexpected asset import') },
        aicc: async () => mode === 'failed-response' ? { status: 'failed', error } : { status: 'running', task_id: 'failed-image' },
        task: async () => ({ phase: 'Terminal', outcome: 'Failed', message: 'task failed', error }),
      })
      const outcome = await runProgram(`export default async function main(aiws) {
        aiws.declare({ inputs: {}, ai: { image: { max_calls: 1 } } })
        await aiws.ai.image({ prompt: 'landscape', quality: 'high' })
      }`, { 'context/inputs.json': { text: '{}' }, 'request.json': { text: '{}' } }, (op, req) => host.handle(op, req), { timeoutMs: 5000 })
      const results = outcome.results as { error?: string; error_code?: string }
      const state = JSON.parse(wish_main_status(JSON.stringify({ status: 'stale', analysis_status: 'ready', input_problems: [] }), JSON.stringify({ last_failed: { stage: 'run', error: { code: 'DEPENDENCY_UNAVAILABLE', sub_code: results.error_code } } })))
      return { results, state }
    }, mode)
    expect(result.results.error_code).toBe('AI_UNSUPPORTED_OPERATION')
    expect(result.results.error).toContain('selected-model@test-provider')
    expect(result.state.main).toBe('cannot_run')
    expect(result.state.next_action).toBe('fix')
  })
}
