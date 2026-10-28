import { expect, test } from '@playwright/test'
import { openFromAllApps } from '../e2e/fixtures/desktop'

test.use({
  baseURL: process.env.BUCKYOS_UI_DV_BASE_URL || `https://${process.env.BUCKYOS_TEST_ZONE_HOST || 'test.buckyos.io'}`,
  trace: 'off',
})

const accounts = [
  {
    username: process.env.BUCKYOS_TEST_MEMBER_USER || 'bob',
    password: process.env.BUCKYOS_TEST_MEMBER_PASSWORD || '',
    type: 'user',
    label: 'User',
  },
  {
    username: process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest',
    password: process.env.BUCKYOS_TEST_ADMIN_PASSWORD || '',
    type: 'admin',
    label: 'Admin',
  },
]

for (const mobile of [false, true]) {
  test.describe(mobile ? 'mobile account role' : 'desktop account role', () => {
    test.use({
      viewport: mobile ? { width: 375, height: 812 } : { width: 1440, height: 900 },
      isMobile: mobile,
      hasTouch: mobile,
    })

    for (const account of accounts) {
      test(`${account.type} is shown as ${account.label} rather than Owner`, async ({ page }, info) => {
        test.skip(!account.password, 'Set BUCKYOS_TEST_MEMBER_PASSWORD and BUCKYOS_TEST_ADMIN_PASSWORD.')
        const errors: string[] = []
        page.on('pageerror', (error) => errors.push(error.message))
        page.on('console', (message) => {
          if (message.type() !== 'error') return
          const location = message.location().url
          if (location.endsWith('/sso_refresh') && message.text().includes('401 (Unauthorized)')) return
          if (message.text() === 'An SSL certificate error occurred when fetching the script.') {
            info.annotations.push({
              type: 'known-environment-error',
              description: 'The DV certificate prevents Chromium service worker registration.',
            })
            return
          }
          errors.push(`${message.text()} (${location})`)
        })

        try {
          await page.goto('/')
          await page.getByLabel('Username').fill(account.username)
          await page.getByLabel('Password').fill(account.password)
          await page.getByRole('button', { name: 'Sign In', exact: true }).click()
          await expect(page.getByRole('button', { name: 'BuckyOS' }).first()).toBeVisible()

          const detailResponse = page.waitForResponse((response) => {
            if (!response.url().includes('/kapi/control-panel')) return false
            const body = response.request().postDataJSON()
            return body?.method === 'user.get'
              && (!body.params?.user_id || body.params.user_id === account.username)
          })
          await openFromAllApps(page, 'users-agents')
          const app = mobile
            ? page
            : page.getByTestId('window-users-agents')
          const detail = (await (await detailResponse).json()).result
          expect(detail.user_type).toBe(account.type)
          const displayName = detail.profile?.display_name || detail.show_name || account.username
          const selfCard = app.getByRole('button').filter({ has: page.getByText(displayName, { exact: true }) })
          await expect(selfCard).toBeVisible()
          await expect(selfCard.getByText('Owner', { exact: true })).toHaveCount(0)
          if (mobile || !detail.profile?.bio) {
            await expect(selfCard.getByText(account.label, { exact: true })).toBeVisible()
          }
          await page.screenshot({ path: info.outputPath('account-card.png'), fullPage: true })

          await selfCard.click()
          const main = app.getByRole('main').last()
          await expect(main.getByRole('heading', { name: displayName, exact: true })).toBeVisible()
          await expect(main.getByText(account.label, { exact: true })).toBeVisible()
          await expect(main.getByText('Owner', { exact: true })).toHaveCount(0)
          await page.screenshot({ path: info.outputPath('account-detail.png'), fullPage: true })
          expect(errors).toEqual([])
        } finally {
          await info.attach('console-errors', {
            body: JSON.stringify(errors, null, 2),
            contentType: 'application/json',
          })
        }
      })
    }
  })
}
