import { imageModelCapabilities } from '../../src/app/aiworkspace/executor/infer.ts'

function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message)
}

Deno.test('coding hints contain bounded image mappings without unrelated provider data', async () => {
  const text = await imageModelCapabilities(async (method, params) => {
    assert(method === 'models.list' && Object.keys(params).length === 0, 'only read the visible model inventory')
    return { models: [
      { exact_model: 'text@provider', capabilities: { tool_call: true } },
      ...Array.from({ length: 40 }, (_, n) => ({ exact_model: `image-${n}@provider`, secret: 'not-for-the-prompt',
        capabilities: { 'image.img2img.aspect_ratio': true, 'image.img2img.quality': false, tool_call: true } })),
    ] }
  })
  assert(text.includes('image-31@provider') && !text.includes('image-32@provider'), 'limit inventory hints to 32 models')
  assert(text.includes('image.img2img.aspect_ratio'), 'include declared mappings')
  assert(!text.includes('image.img2img.quality') && !text.includes('text@provider'), 'omit false and unrelated capabilities')
  assert(!text.includes('not-for-the-prompt'), 'never dump full provider objects into the prompt')
})

Deno.test('unavailable capability inventory is unknown rather than an empty inventory', async () => {
  const text = await imageModelCapabilities(async () => { throw new Error('offline') })
  assert(text.includes('could not be read') && text.includes('does not establish'), 'keep unavailable distinct from no model')
})
