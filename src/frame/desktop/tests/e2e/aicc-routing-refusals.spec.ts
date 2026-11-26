import { expect, test } from '@playwright/test'

test('missing model and denied access keep distinct workspace errors', async ({ page }) => {
  await page.route('**/__routing-test', (route) => route.fulfill({ contentType: 'text/html', body: '<!doctype html>' }))
  await page.goto('/__routing-test')
  const codes = await page.evaluate(async () => {
    const path = '/src/app/aiworkspace/executor/infer.ts'
    const { refusalCode } = await import(path)
    return ['no_candidate_model', 'resource_not_permitted', 'quota_exhausted', 'unknown'].map(refusalCode)
  })
  expect(codes).toEqual(['AI_NO_CANDIDATE_MODEL', 'AI_RESOURCE_NOT_PERMITTED', 'AI_QUOTA_EXHAUSTED', undefined])
})
