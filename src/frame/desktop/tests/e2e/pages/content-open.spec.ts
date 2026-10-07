import { test, expect, type Page } from '@playwright/test'
import { readdir, readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

async function files(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.route(/http:\/\/(notes|reader)-fixture\.localhost:4173\//, route => route.fulfill({ contentType: 'text/html', body: `<!doctype html><title>Handler fixture</title><pre id="source"></pre><script>
    const nonce = new URLSearchParams(location.hash.slice(1)).get('bfp');
    const send = (type, payload, replyTo) => parent.postMessage({ protocol: 'buckyos.app-frame', version: 1, type, payload, replyTo }, 'http://127.0.0.1:4173');
    addEventListener('message', e => {
      if (e.origin !== 'http://127.0.0.1:4173' || e.source !== parent) return;
      const m = e.data; const request = m.type === 'frame.init' ? m.payload.launch : m.payload.request;
      if (request) { document.querySelector('#source').textContent = request.source.path; send('window.setTitle', { title: 'Fixture: ' + request.source.path.split('/').pop(), dirty: false }); }
      if (m.type === 'frame.open') send('frame.openResult', { accepted: true }, m.id);
      if (m.type === 'frame.beforeClose') send('frame.beforeCloseResult', { decision: 'allow' }, m.id);
    });
    send('frame.hello', { nonce, appCapabilities: ['open', 'beforeClose'] });
  </script>` }))
  await page.goto('/?scenario=normal&contentApps=1')
  await page.getByTestId('desktop-app-files').click()
  await page.locator('aside').getByRole('button', { name: /Documents/ }).first().click()
  return page.getByRole('cell', { name: /^Kyoto Trip Plan\.md/ }).first()
}

test('generic handler opens from Files, receives session, reuses frame and closes', async ({ page }) => {
  const file = await files(page)
  await file.dblclick()
  const frame = page.getByTestId('web-app-frame')
  await expect(frame).toHaveCount(1)
  await expect(frame).toHaveAttribute('src', /notes-fixture.*src=.*Kyoto/)
  await expect(page.frameLocator('[data-testid="web-app-frame"]').locator('#source')).toContainText('Kyoto Trip Plan.md')
  await expect(page.getByTestId('window-notes-fixture')).toContainText('Fixture: Kyoto Trip Plan.md')
  await page.getByTestId('window-files').click({ position: { x: 10, y: 10 } })
  await file.dblclick()
  await expect(frame).toHaveCount(1)
})

test('Open with lists both handlers and Preview; defaults and disabling affect double click', async ({ page }) => {
  const file = await files(page)
  await file.click({ button: 'right' })
  await page.getByRole('menuitem', { name: /^Open with/ }).click()
  await expect(page.getByRole('menuitem', { name: 'Notebook Fixture', exact: true })).toBeVisible()
  await expect(page.getByRole('menuitem', { name: 'Reader Fixture', exact: true })).toBeVisible()
  await expect(page.getByRole('menuitem', { name: 'Preview', exact: true })).toBeVisible()
  await page.getByRole('menuitem', { name: 'Choose default app…', exact: true }).click()
  const chooser = page.getByRole('dialog', { name: 'Choose default app…' })
  await chooser.getByLabel('Reader Fixture').check()
  await chooser.getByRole('button', { name: 'Open', exact: true }).click()
  await expect(page.getByTestId('web-app-frame')).toHaveAttribute('src', /reader-fixture/)
  await page.reload()
  await page.getByTestId('desktop-app-files').click()
  await page.locator('aside').getByRole('button', { name: /Documents/ }).first().click()
  await page.getByRole('cell', { name: /^Kyoto Trip Plan\.md/ }).first().dblclick()
  await expect(page.getByTestId('web-app-frame')).toHaveAttribute('src', /reader-fixture/)
  await page.evaluate(() => localStorage.setItem('mock.content.defaults', JSON.stringify({ schema_version: 1, defaults: {}, disabled: ['notes.fixture.bns.did@owner#text', 'reader.fixture.bns.did@owner#text'] })))
  await page.reload()
  await page.getByTestId('desktop-app-files').click()
  await page.locator('aside').getByRole('button', { name: /Documents/ }).first().click()
  await page.getByRole('cell', { name: /^Kyoto Trip Plan\.md/ }).first().dblclick()
  await expect(page.getByTestId('window-preview')).toBeVisible()
})

test('Desktop contains no built-in dependency on the editor identity', async () => {
  const root = fileURLToPath(new URL('../../../src/', import.meta.url))
  const forbidden = ['text' + '-editor.buckyos.bns.did', 'text' + '-editor.']
  async function inspect(dir: string): Promise<void> {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const file = path.join(dir, entry.name)
      if (entry.isDirectory()) await inspect(file)
      else if (/\.[cm]?[jt]sx?$/.test(file)) for (const token of forbidden) expect(await readFile(file, 'utf8'), file).not.toContain(token)
    }
  }
  await inspect(root)
})

test('clicking inside a web app iframe raises its window', async ({ page }) => {
  const file = await files(page)
  await file.dblclick()
  const frame = page.getByTestId('web-app-frame')
  await expect(page.frameLocator('[data-testid="web-app-frame"]').locator('#source')).toContainText('Kyoto Trip Plan.md')
  const z = async (id: string) => Number(await page.getByTestId(`window-${id}`).evaluate(el => (el as HTMLElement).style.zIndex))
  // Raise Files over the handler window, then click the part of the iframe that is still visible.
  await page.getByTestId('window-files').click({ position: { x: 10, y: 10 } })
  expect(await z('files')).toBeGreaterThan(await z('notes-fixture'))
  // Pick a point well inside the iframe (away from the window's resize handles) that the
  // Files window does not cover, so the click really lands in the iframe document.
  const iframeBox = (await frame.boundingBox())!
  const filesBox = (await page.getByTestId('window-files').boundingBox())!
  const inset = 20
  const candidates = [
    { x: iframeBox.x + iframeBox.width - inset, y: iframeBox.y + iframeBox.height / 2 },
    { x: iframeBox.x + iframeBox.width / 2, y: iframeBox.y + iframeBox.height - inset },
    { x: iframeBox.x + iframeBox.width - inset, y: iframeBox.y + iframeBox.height - inset },
  ]
  const covered = (p: { x: number; y: number }) => p.x >= filesBox.x && p.x <= filesBox.x + filesBox.width && p.y >= filesBox.y && p.y <= filesBox.y + filesBox.height
  const point = candidates.find(p => !covered(p))
  expect(point, 'the handler window must stick out from under the Files window').toBeTruthy()
  await page.mouse.click(point!.x, point!.y)
  await expect.poll(() => page.evaluate(() => document.activeElement?.getAttribute('data-testid'))).toBe('web-app-frame')
  await expect.poll(async () => (await z('notes-fixture')) > (await z('files'))).toBe(true)
  // The iframe element itself must not have been replaced or reloaded by the raise.
  await expect(frame).toHaveCount(1)
  await expect(page.frameLocator('[data-testid="web-app-frame"]').locator('#source')).toContainText('Kyoto Trip Plan.md')
})
