import { createHash } from 'node:crypto'
import { readFileSync, readdirSync } from 'node:fs'
import { test, expect } from './fixtures'

const digest = (bytes: Buffer) => createHash('sha256').update(bytes).digest('hex')

test('PDF decoder resources are served by the production build', async ({ request }) => {
  const base = `http://localhost:${process.env.AIWS_E2E_PREVIEW_PORT}`
  const dir = new URL('../../node_modules/pdfjs-dist/wasm/', import.meta.url)
  const files = readdirSync(dir).filter(name => /\.(wasm|js)$|^LICENSE/.test(name))
  expect(files).toContain('jbig2.wasm')
  expect(files).toContain('jbig2_nowasm_fallback.js')
  for (const name of files) {
    const response = await request.get(`${base}/aiworkspace/pdfjs/wasm/${name}`)
    expect(response.status(), name).toBe(200)
    expect(digest(await response.body()), name).toBe(digest(readFileSync(new URL(name, dir))))
    if (name.endsWith('.wasm')) expect(response.headers()['content-type']).toContain('application/wasm')
    if (name.endsWith('.js')) expect(response.headers()['content-type']).toMatch(/javascript/)
  }
  const worker = await request.get(`${base}/sw.js`)
  expect(worker.status()).toBe(200)
  expect(await worker.text()).not.toContain('aiworkspace/pdfjs/wasm/')
})
