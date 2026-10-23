import { test, expect, type Page } from '@playwright/test'
import { routeAppHost } from '../fixtures/desktop'

// The fixture App is fulfilled by Playwright (no address) and frames the dev server on 127.0.0.1; Chrome's Local
// Network Access check would treat that as a public page reaching into the local network. In a Zone, the App host
// and the Zone root resolve to the same gateway.
test.use({ launchOptions: { args: ['--disable-features=LocalNetworkAccessChecks,PrivateNetworkAccessForNavigations,PrivateNetworkAccessRespectPreflightResults'] } })

/**
 * Issue #675: an App shows the system Preview inside its own UI (`/embed/preview`, `buckyos.preview-embed`) and
 * hands "Open with…" to the shell (`content.open` with target `chooser`). The fixture App is a plain page that
 * speaks both protocols on the wire, as any third-party App would.
 */
const fixture = (source: string) => `<!doctype html><title>Embed fixture</title><output id="state"></output><output id="focus">0</output><output id="titles">0</output>
<iframe id="embed" style="width:100%;height:420px;border:0"></iframe><script>
  const shell = 'http://127.0.0.1:4173'
  const nonce = new URLSearchParams(location.hash.slice(1)).get('bfp')
  const embedNonce = crypto.randomUUID()
  const frame = (type, payload, replyTo) => parent.postMessage({ protocol: 'buckyos.app-frame', version: 1, type, payload, replyTo }, shell)
  const embed = document.querySelector('#embed')
  let focusMessages = 0, titles = 0
  addEventListener('message', e => {
    const m = e.data
    if (e.source === parent && e.origin === shell && m.protocol === 'buckyos.app-frame') {
      if (m.type === 'frame.init') { embed.src = shell + '/embed/preview#bpe=' + embedNonce + '&po=' + encodeURIComponent(location.origin) }
      if (m.type === 'frame.focusChanged') { document.querySelector('#focus').textContent = String(++focusMessages); frame('window.setTitle', { title: 'Embed fixture ' + (titles % 2), dirty: false }); document.querySelector('#titles').textContent = String(++titles) }
      if (m.type === 'frame.beforeClose') frame('frame.beforeCloseResult', { decision: 'allow' }, m.id)
      if (m.type === 'frame.open') frame('frame.openResult', { accepted: true }, m.id)
    }
    if (e.source === embed.contentWindow && e.origin === shell && m.protocol === 'buckyos.preview-embed') {
      if (m.type === 'embed.hello' && m.payload.nonce === embedNonce) embed.contentWindow.postMessage({ protocol: 'buckyos.preview-embed', version: 1, type: 'embed.init', payload: { nonce: embedNonce, source: ${JSON.stringify({ kind: 'cyfs-path', path: source })}, theme: 'dark', locale: 'zh-CN' } }, shell)
      if (m.type === 'embed.state') document.querySelector('#state').textContent = m.payload.status + (m.payload.error ? ':' + m.payload.error.kind : '')
      if (m.type === 'embed.openWith') frame('content.open', { request: m.payload.request, target: 'chooser', hints: m.payload.hints })
    }
  })
  frame('frame.hello', { nonce, appCapabilities: ['open', 'beforeClose'] })
</script>`

async function openFixture(page: Page, source: string) {
  await page.setViewportSize({ width: 1440, height: 900 })
  await routeAppHost(page, /http:\/\/(notes|reader)-fixture\.localhost:4173\//, fixture(source))
  await page.goto('/?scenario=normal&contentApps=1')
  await page.getByTestId('desktop-app-files').click()
  await page.locator('aside').getByRole('button', { name: /Documents/ }).first().click()
  await page.getByRole('cell', { name: /^Kyoto Trip Plan\.md/ }).first().dblclick()
  await expect(page.getByTestId('window-notes-fixture')).toBeVisible()
  const app = page.frameLocator('[data-testid="web-app-frame"]')
  return { app, embed: app.frameLocator('#embed') }
}

test('an App shows the system Preview inline and its Open with goes to the shell, without offering the App itself', async ({ page }) => {
  const { app, embed } = await openFixture(page, 'cyfs:///samples/release-notes.md')
  await expect(embed.getByTestId('content-preview')).toHaveAttribute('data-status', 'ready', { timeout: 15_000 })
  await expect(app.locator('#state')).toHaveText('ready')
  await expect(embed.getByTestId('content-preview-text')).toContainText('What changed in this build')
  // The embedding App decides the theme and language; the shell's own stored choice is untouched.
  await expect(embed.locator('html')).toHaveAttribute('data-theme', 'dark')
  await expect(embed.locator('html')).toHaveAttribute('lang', 'zh-CN')
  expect(await page.evaluate(() => localStorage.getItem('buckyos.prototype.locale.v1'))).not.toBe('zh-CN')
  // No Preview App window: the content stays inside the App.
  await expect(page.getByTestId('window-preview')).toHaveCount(0)

  await embed.getByTestId('content-preview').hover()
  await embed.getByTestId('content-preview-open-with').click()
  const sheet = page.getByRole('dialog', { name: /Open with|打开方式|使用其他应用打开/ })
  await expect(sheet).toBeVisible()
  await expect(sheet.getByRole('button', { name: 'Reader Fixture', exact: true })).toBeVisible()
  await expect(sheet.getByRole('button', { name: 'Preview', exact: true })).toBeVisible()
  await expect(sheet.getByRole('button', { name: 'Notebook Fixture', exact: true })).toHaveCount(0)
  // Cancelling keeps the App and its inline preview as they were.
  await sheet.getByRole('button', { name: /Close|关闭/ }).click()
  await expect(page.getByTestId('preview-open-with')).toHaveCount(0)
  await expect(embed.getByTestId('content-preview')).toHaveAttribute('data-status', 'ready')

  await embed.getByTestId('content-preview').hover()
  await embed.getByTestId('content-preview-open-with').click()
  await page.getByRole('dialog', { name: /Open with|打开方式|使用其他应用打开/ }).getByRole('button', { name: 'Reader Fixture', exact: true }).click()
  await expect(page.getByTestId('window-reader-fixture')).toBeVisible()
})

test('Preview shows its own unsupported state inline and the window does not ping-pong focus and title', async ({ page }) => {
  const { app, embed } = await openFixture(page, 'cyfs:///samples/keynote.pptx')
  await expect(app.locator('#state')).toHaveText('error:unsupported', { timeout: 15_000 })
  await expect(embed.getByTestId('content-preview')).toHaveAttribute('data-status', 'error')
  // Every focus change answered with a title change must settle, not loop.
  await page.waitForTimeout(1500)
  const focus = Number(await app.locator('#focus').textContent())
  expect(focus).toBeLessThan(5)
  await page.waitForTimeout(1000)
  expect(Number(await app.locator('#focus').textContent())).toBe(focus)
  await page.getByTestId('window-files').click({ position: { x: 10, y: 10 } })
  await expect.poll(async () => Number(await app.locator('#focus').textContent())).toBe(focus + 1)
})

test('the system entry opens the chooser from a link and leaves out the requesting App', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto(`/?scenario=normal&contentApps=1&open=${encodeURIComponent('cyfs:///home/Documents/Kyoto Trip Plan.md')}&intent=chooser&name=${encodeURIComponent('Kyoto Trip Plan.md')}&from=notes.fixture.bns.did`)
  const sheet = page.getByRole('dialog', { name: /Open with|打开方式|使用其他应用打开/ })
  await expect(sheet).toBeVisible()
  await expect(sheet.getByRole('button', { name: 'Reader Fixture', exact: true })).toBeVisible()
  await expect(sheet.getByRole('button', { name: 'Notebook Fixture', exact: true })).toHaveCount(0)
  await sheet.getByRole('button', { name: 'Preview', exact: true }).click()
  await expect(page.getByTestId('window-preview')).toBeVisible()
})

test('the embed page refuses to show content for a page outside the Zone', async ({ page }) => {
  await page.route('http://outside.example/', route => route.fulfill({ contentType: 'text/html', body: `<iframe id="embed" src="http://127.0.0.1:4173/embed/preview#bpe=x&po=${encodeURIComponent('http://outside.example')}"></iframe>` }))
  await page.goto('http://outside.example/')
  await expect(page.frameLocator('#embed').getByRole('alert')).toBeVisible({ timeout: 15_000 })
})
