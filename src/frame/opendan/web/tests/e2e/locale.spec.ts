import { expect, test } from '@playwright/test'

test('the standalone agent page honors a regional language override', async ({ page }) => {
  await page.goto('/?data=mock&lang=zh-Hant-HK')
  await expect(page.locator('html')).toHaveAttribute('lang', 'zh-TW')
  await expect(page.getByRole('link', { name: /詳細設定/ })).toBeVisible()
})

test('the agent window follows Desktop language changes without losing navigation', async ({ page }) => {
  await page.route('**/desktop-test', (route) => route.fulfill({
    contentType: 'text/html',
    body: `<iframe src="/?data=mock&lang=en#bfp=locale-test"></iframe>
<script>
const frame = document.querySelector('iframe')
const send = (type, payload) => frame.contentWindow.postMessage({ protocol: 'buckyos.app-frame', version: 1, type, payload }, location.origin)
addEventListener('message', event => {
  if (event.source === frame.contentWindow && event.data.type === 'frame.hello') {
    send('frame.init', { nonce: 'locale-test', windowId: 'agent', theme: {mode: 'light'}, formFactor: 'desktop', locale: 'fr', shellCapabilities: [] })
  }
})
</script>
<button onclick="send('frame.localeChanged', {locale: 'ar'})">Arabic</button>`,
  }))
  await page.goto('/desktop-test')
  const agent = page.frameLocator('iframe')
  await expect(agent.locator('html')).toHaveAttribute('lang', 'fr')
  await agent.locator('footer a[href="#/sessions"]').click()
  await expect(agent.getByTestId('active-panel')).toBeVisible()
  await page.getByRole('button', { name: 'Arabic' }).click()
  await expect(agent.locator('html')).toHaveAttribute('lang', 'ar')
  await expect(agent.locator('html')).toHaveAttribute('dir', 'rtl')
  await expect(agent.getByTestId('active-panel')).toBeVisible()
  await expect(agent.getByRole('button', { name: 'تحديث' })).toBeVisible()
})
