import { createHash } from 'node:crypto'
import { readFileSync, readdirSync } from 'node:fs'
import { test, expect } from './fixtures'

const digest = (bytes: Buffer) => createHash('sha256').update(bytes).digest('hex')

test('PDF decoder resources are served by the production build', async ({ request }) => {
  const base = `http://localhost:${process.env.AIWS_E2E_PREVIEW_PORT}`
  const dir = new URL('../../node_modules/pdfjs-dist/wasm/', import.meta.url)
  const files = [
    'jbig2.wasm', 'jbig2_nowasm_fallback.js', 'openjpeg.wasm', 'openjpeg_nowasm_fallback.js', 'qcms_bg.wasm',
    ...readdirSync(dir).filter(name => name.startsWith('LICENSE')),
  ]
  for (const name of files) {
    const response = await request.get(`${base}/aiworkspace/pdfjs/wasm/${name}`)
    expect(response.status(), name).toBe(200)
    expect(digest(await response.body()), name).toBe(digest(readFileSync(new URL(name, dir))))
    if (name.endsWith('.wasm')) expect(response.headers()['content-type']).toContain('application/wasm')
    if (name.endsWith('.js')) expect(response.headers()['content-type']).toMatch(/javascript/)
  }
  for (const name of ['quickjs-eval.js', 'quickjs-eval.wasm']) {
    const response = await request.get(`${base}/aiworkspace/pdfjs/wasm/${name}`)
    expect(digest(await response.body()), name).not.toBe(digest(readFileSync(new URL(name, dir))))
  }
  for (const name of ['CGATS001Compat-v2-micro.icc', 'LICENSE']) {
    const response = await request.get(`${base}/aiworkspace/pdfjs/iccs/${name}`)
    expect(response.status(), name).toBe(200)
    const local = readFileSync(new URL(`../../node_modules/pdfjs-dist/iccs/${name}`, import.meta.url))
    expect(digest(await response.body()), name).toBe(digest(local))
  }
  const worker = await request.get(`${base}/sw.js`)
  expect(worker.status()).toBe(200)
  expect(await worker.text()).not.toMatch(/aiworkspace\/pdfjs\/(wasm|iccs)\//)
})
