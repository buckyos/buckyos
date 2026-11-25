import { expect, test } from '@playwright/test'
import { routeAppHost } from '../fixtures/desktop'

test("apps get the Zone's color with their theme, and hear when it changes", async ({ page, baseURL }) => {
  const origin = new URL(baseURL!).origin
  const port = new URL(baseURL!).port
  await page.setViewportSize({ width: 1440, height: 900 })
  await routeAppHost(page, new RegExp(`http://notes-fixture\\.localhost:${port}/`), `<!doctype html><title>Accent fixture</title><output id="themes"></output><script>
    const nonce = new URLSearchParams(location.hash.slice(1)).get('bfp');
    const send = (type, payload, replyTo) => parent.postMessage({ protocol: 'buckyos.app-frame', version: 1, type, payload, replyTo }, '${origin}');
    addEventListener('message', e => {
      if (e.origin !== '${origin}' || e.source !== parent) return;
      const m = e.data, theme = m.type === 'frame.init' ? m.payload.theme : m.type === 'frame.themeChanged' ? m.payload : undefined;
      if (theme) document.querySelector('#themes').textContent += theme.mode + ':' + (theme.accent ?? '-') + ' ';
      if (m.type === 'frame.beforeClose') send('frame.beforeCloseResult', { decision: 'allow' }, m.id);
    });
    send('frame.hello', { nonce, appCapabilities: ['open', 'beforeClose'] });
  </script>`)
  await page.addInitScript(() => {
    if (window.top === window) localStorage.setItem('buckyos.desktop.accent', '#b8437f')
  })
  await page.goto('/?scenario=normal&contentApps=1')
  await page.getByTestId('desktop-app-files').click()
  await page.getByTestId('window-files').getByRole('cell', { name: 'Documents', exact: true }).dblclick()
  await page.getByRole('cell', { name: /^Kyoto Trip Plan\.md/ }).first().dblclick()
  const themes = page.frameLocator('[data-testid="web-app-frame"]').locator('#themes')
  await expect(themes).toContainText('light:#b8437f')

  for (const window of ['window-notes-fixture', 'window-files']) await page.getByTestId(window).getByRole('button', { name: 'Minimize', exact: true }).click()
  await page.getByTestId('desktop-app-settings').click()
  await page.getByTestId('window-settings').getByRole('button', { name: 'Appearance', exact: true }).click()
  await page.getByTestId('settings-accent-reset').click()
  await expect(themes).toContainText('light:-')
})
