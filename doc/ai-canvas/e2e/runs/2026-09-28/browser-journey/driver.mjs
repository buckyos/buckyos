import fs from 'node:fs/promises'
import { createRequire } from 'node:module'
const root = '/root/app/buckyos/.work/canvas-journey-20260928'
const require = createRequire('/root/app/buckyos/buckyos/src/frame/desktop/package.json')
const { chromium, expect } = require('@playwright/test')
const browser = await chromium.launch({ headless: true, executablePath: '/root/.cache/ms-playwright/chromium-1223/chrome-linux64/chrome', timeout: 180000 })
const context = await browser.newContext({ viewport: { width: 1600, height: 1000 }, acceptDownloads: true })
await context.tracing.start({ screenshots: true, snapshots: true, sources: true })
const page = await context.newPage()
page.setDefaultTimeout(15000)
const errors = []
page.on('pageerror', e => errors.push(e.message))
const env = { root, fs, browser, context, page, expect }
await fs.writeFile(`${root}/ready.json`, JSON.stringify({ browser: browser.version(), playwright: require('@playwright/test/package.json').version }))
let i = 1
const start = Date.now()
while (Date.now() - start < 3600000) {
  const key = String(i).padStart(2,'0')
  try { await fs.access(`${root}/steps/${key}.ready`) } catch { await new Promise(r => setTimeout(r,250)); continue }
  const result = { step: key, startedAt: new Date().toISOString() }
  try {
    const mod = await import(`${root}/steps/${key}.mjs`)
    result.returned = await mod.default(env)
  } catch(e) { result.error = e.stack }
  result.finishedAt = new Date().toISOString()
  try {
    result.ui = await env.page.locator('body').ariaSnapshot({ timeout: 10000 })
    await env.page.screenshot({ path: `${root}/evidence/${key}.png` })
  } catch(e) { result.snapshotError = e.message }
  result.pageErrors = [...errors]
  await fs.writeFile(`${root}/evidence/${key}.json`, JSON.stringify(result,null,2))
  if (env.done) break
  i++
}
await context.tracing.stop({ path: `${root}/evidence/author-trace.zip` })
if (env.receiverContext) await env.receiverContext.tracing.stop({ path: `${root}/evidence/receiver-trace.zip` })
await browser.close()
await fs.writeFile(`${root}/driver.exit`,'0\n')
