import { expect, test } from '@playwright/test'

test('image inventory hints and unsupported parameter errors survive browser SDK handling', async ({ page }) => {
  await page.route('**/__image_contract_test', route => route.fulfill({ contentType: 'text/html', body: '<html></html>' }))
  await page.goto('/__image_contract_test')
  const result = await page.evaluate(async () => {
    const { imageModelCapabilities, classified } = await import('/src/app/aiworkspace/executor/infer.ts')
    const { providerErrorFromRpc } = await import('/node_modules/buckyos/dist/xllm.mjs')
    const text = await imageModelCapabilities(async () => ({ models: Array.from({ length: 40 }, (_, n) => ({
      exact_model: `image-${n}@provider`, secret: 'private-value', capabilities: { 'image.img2img.aspect_ratio': true, 'image.img2img.quality': false },
    })) }))
    const error = classified(providerErrorFromRpc('image', { code: 'unsupported_operation', message: 'negative_prompt for image@test' }))
    return { text, code: error.code }
  })
  expect(result.text).toContain('image-31@provider')
  expect(result.text).not.toContain('image-32@provider')
  expect(result.text).not.toContain('private-value')
  expect(result.text).not.toContain('image.img2img.quality')
  expect(result.code).toBe('AI_UNSUPPORTED_OPERATION')
})
