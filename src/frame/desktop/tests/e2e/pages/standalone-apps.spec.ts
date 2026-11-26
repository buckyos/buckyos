import { expect, test, type Page } from '@playwright/test'

interface SystemApp { id: string; names: Record<string, string>; icon: string; path: string; show_in_launcher: boolean; audience: string; form_factors: string[]; window: Record<string, unknown> }

async function systemApps(page: Page): Promise<SystemApp[]> {
  const response = await page.request.get('/system-apps.json')
  expect(response.ok()).toBe(true)
  const manifest = await response.json() as { version: number; apps: SystemApp[] }
  expect(manifest.version).toBe(1)
  return manifest.apps
}

async function expectNoShell(page: Page) {
  await expect(page.getByTestId('desktop-workspace')).toHaveCount(0)
  await expect(page.locator('[data-testid^="window-drag-"]')).toHaveCount(0)
}

test('system-apps.json lists every system app with its names, icon image, page and window', async ({ page }) => {
  const apps = await systemApps(page)
  expect(apps.map((app) => app.id)).toEqual(expect.arrayContaining(['files', 'settings', 'task-center', 'ai-center', 'messagehub', 'homestation', 'aiworkspace', 'preview']))
  expect(apps.map((app) => app.id)).not.toContain('diagnostics')
  expect(apps.map((app) => app.id)).not.toContain('demos')
  for (const app of apps) {
    for (const locale of ['en', 'zh-CN', 'zh-TW', 'ja', 'ko', 'fr', 'de', 'es', 'ru']) expect(app.names[locale], `${app.id} ${locale}`).toBeTruthy()
    const icon = await page.request.get(app.icon)
    expect(icon.headers()['content-type']).toContain('svg')
    const svg = await icon.text()
    expect(svg, app.id).toMatch(/^<svg [^>]*viewBox=/)
    expect(svg, app.id).not.toMatch(/var\(|currentColor|oklch|color-mix/)
  }
  const byId = new Map(apps.map((app) => [app.id, app]))
  expect(byId.get('messagehub')).toMatchObject({ path: '/messagehub', show_in_launcher: true, audience: 'all', window: { instance: 'single', default_mode: 'windowed', mobile_header: 'compact' } })
  expect(byId.get('preview')).toMatchObject({ path: '/app/preview', window: { instance: 'multi', resizable: true } })
  expect(byId.get('task-center')).toMatchObject({ path: '/taskcenter' })
  expect(byId.get('agent-setup')).toMatchObject({ show_in_launcher: false, audience: 'agent_user' })
})

for (const viewport of [{ name: 'desktop', width: 1100, height: 720 }, { name: 'phone', width: 390, height: 844 }]) {
  test(`every system app has a page of its own without the desktop (${viewport.name})`, async ({ page }) => {
    test.setTimeout(180_000)
    await page.setViewportSize(viewport)
    for (const app of await systemApps(page)) {
      await page.goto(app.path)
      // The guide goes on to the setup wizard.
      const shown = app.id === 'agent-guide' ? 'agent-setup' : app.id
      await expect(page.getByTestId(`standalone-${shown}`), app.id).toBeVisible({ timeout: 30_000 })
      await expect(page, app.id).not.toHaveTitle(/^(alice|BuckyOS)$/)
      await expectNoShell(page)
      const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
      expect(overflow, `${app.id} scrolls sideways`).toBeLessThanOrEqual(0)
    }
  })
}

test('an address opens a page inside the app, and the address follows the app', async ({ page }) => {
  test.setTimeout(90_000)
  await page.goto('/app/settings/privacy')
  await expect(page.getByRole('heading', { name: 'Public Access' })).toBeVisible()
  await expect(page).toHaveTitle(/^Settings · /)
  await page.getByRole('button', { name: 'Zone Config' }).click()
  await expect(page).toHaveURL(/\/app\/settings\/zone$/)
  await page.reload()
  await expect(page.getByTestId('settings-zone-did')).toBeVisible()

  await page.goto('/app/ai-center/usage')
  await expect(page.getByTestId('standalone-ai-center')).toContainText('Usage')
  await page.goto('/app/files?path=%2Fhome%2FDocuments')
  await expect(page.getByRole('cell', { name: /^Kyoto Trip Plan\.md/ }).first()).toBeVisible({ timeout: 20_000 })
  await expect(page).toHaveURL(/\/app\/files\?path=%2Fhome%2FDocuments$/)
  await page.goto(`/app/preview?open=${encodeURIComponent('cyfs:///home/Documents/Kyoto Trip Plan.md')}`)
  await expect(page).toHaveTitle(/^Kyoto Trip Plan\.md · /)
  await page.goto('/app/app-service/install')
  await expect(page).toHaveURL(/\/app\/app-service\/install$/)
  await page.goto('/app/my-network/requests')
  await expect(page).toHaveURL(/\/app\/my-network\/requests$/)
  await page.goto('/homestation?new=post')
  await expect(page.getByTestId('homestation')).toContainText('New post')
  await expect(page).toHaveURL(/\/homestation$/)
  // An app with a route of its own is shown there; what has no page goes to the desktop.
  await page.goto('/app/task-center?taskid=nope')
  await expect(page).toHaveURL(/\/taskcenter\?taskid=nope$/)
  await page.goto('/app/diagnostics')
  await expect(page.getByTestId('desktop-workspace')).toBeVisible()
})

test('the sheet for adding files opens from its address on a phone', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await page.goto('/app/files/upload')
  await expect(page.getByText('Add to this folder')).toBeVisible()
  await expect(page).toHaveURL(/\/app\/files$/)
})

test('a phone page has its title and Back, and goes home from its first page', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await page.goto('/app/settings')
  const bar = page.locator('header').first()
  await expect(bar).toContainText('Settings')
  await page.getByRole('button', { name: 'Privacy' }).click()
  await expect(page).toHaveURL(/\/app\/settings\/privacy$/)
  await bar.getByRole('button', { name: 'Back' }).click()
  await expect(page).toHaveURL(/\/app\/settings$/)
  await bar.getByRole('button', { name: 'Back' }).click()
  await expect(page.getByTestId('desktop-workspace')).toBeVisible()
})

test('opening another app from a page opens that app’s own page', async ({ page, context }) => {
  await page.goto('/app/my-content/public')
  const opened = context.waitForEvent('page')
  await page.getByRole('button', { name: /Files/ }).first().click()
  const files = await opened
  await expect(files).toHaveURL(/\/app\/files\?path=%2Fsrv%2Fpublish$/)
  await expect(files.getByTestId('standalone-files')).toBeVisible()
  // The same window is the same tab.
  await page.getByRole('button', { name: /Files/ }).first().click()
  await expect.poll(() => context.pages().length).toBe(2)
})

test('a page of the desktop’s root opens what it is asked for', async ({ page }) => {
  await page.goto('/?open-app=settings&path=%2Fprivacy')
  const settings = page.getByTestId('window-settings')
  await expect(settings.getByRole('heading', { name: 'Public Access' })).toBeVisible()
  await expect(page).toHaveURL(/\/(\?scenario=normal)?$/)
  await page.goto('/?open-app=nothing-here')
  await expect(page.getByText('nothing-here')).toBeVisible()
  // A link that leaves the app's own pages is refused.
  await page.goto('/?open-app=settings&path=%2F%2Fevil.example')
  await expect(page.getByTestId('desktop-workspace')).toBeVisible()
  await expect(settings).toHaveCount(0)
  // On a phone, an app that has a route is shown there.
  await page.setViewportSize({ width: 390, height: 844 })
  await page.goto('/?open-app=task-center&path=%2F%3Ftaskid%3Dnope')
  await expect(page).toHaveURL(/\/taskcenter\?taskid=nope$/)
})

// The bridge of the BuckyOS client, with a host that answers the handshake.
interface Client { env?: Record<string, unknown>; init?: Record<string, unknown> }
async function inClient(page: Page, { env = {}, init = {} }: Client = {}) {
  await page.addInitScript(({ env, init }) => {
    const handlers: Array<(message: unknown) => void> = []
    const sent: Array<{ type: string; id?: string; payload: Record<string, unknown> }> = []
    const post = (message: Record<string, unknown>) => handlers.forEach((handler) => handler({ protocol: 'buckyos.app-frame', version: 1, ...message }))
    Object.assign(window, {
      __frame: {
        sent,
        ask: (type: string, payload: unknown) => new Promise((resolve) => {
          const id = crypto.randomUUID()
          const timer = setInterval(() => {
            const reply = (sent as Array<{ replyTo?: string; payload: unknown }>).find((message) => message.replyTo === id)
            if (reply) { clearInterval(timer); resolve(reply.payload) }
          }, 20)
          post({ type, id, payload })
        }),
        tell: (type: string, payload: unknown) => post({ type, payload }),
      },
      BuckyApi: {
        env: { container: 'mobile-app', formFactor: 'mobile', platform: 'android', clientVersion: '1.0.0', capabilities: ['app.open'], ...env },
        on: (event: string, handler: (message: unknown) => void) => { if (event === 'frame.message') handlers.push(handler); return () => {} },
        call: async (name: string, payload: { type: string; id?: string; payload: Record<string, unknown> }) => {
          if (name !== 'frame.post') throw Object.assign(new Error(name), { code: 'unsupported' })
          sent.push(payload)
          if (payload.type === 'frame.hello') setTimeout(() => post({ type: 'frame.init', payload: { nonce: payload.payload.nonce, windowId: 'zone|app|', formFactor: 'mobile', theme: { mode: 'dark' }, locale: 'en', shellCapabilities: ['app.open', 'content.open', 'beforeClose', 'back'], presentation: 'window', container: 'mobile-app', ...init } }), 0)
          if (payload.type === 'app.open') setTimeout(() => post({ type: 'app.openResult', replyTo: payload.id, payload: { ok: true } }), 0)
          return null
        },
      },
    })
  }, { env, init })
}
const sent = (page: Page, type: string) => page.evaluate((type) => (window as unknown as { __frame: { sent: Array<{ type: string; payload: unknown }> } }).__frame.sent.filter((message) => message.type === type).map((message) => message.payload), type)
const ask = (page: Page, type: string, payload: unknown) => page.evaluate(([type, payload]) => (window as unknown as { __frame: { ask: (type: unknown, payload: unknown) => Promise<unknown> } }).__frame.ask(type, payload), [type, payload] as const)

test('in the client a page follows its theme, reports its title and Back, and opens apps through it', async ({ page, context }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await inClient(page)
  await page.goto('/app/my-content/public')
  await expect(page.getByTestId('standalone-my-content')).toHaveAttribute('data-container', 'mobile-app')
  // Not the page's own bar: the client draws the header.
  await expect(page.getByRole('button', { name: 'Back', exact: true })).toHaveCount(0)
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')
  // The client's theme is not the account's: nothing keeps it here.
  expect(await page.evaluate(() => localStorage.getItem('buckyos.prototype.theme.v1'))).not.toBe('dark')
  await expect.poll(() => sent(page, 'window.setTitle')).toContainEqual({ title: 'My Content', dirty: false })
  await expect.poll(() => sent(page, 'header.set')).toContainEqual({ title: 'My Content', back: false })
  expect(await ask(page, 'frame.back', {})).toEqual({ handled: false })
  expect(await ask(page, 'frame.beforeClose', { reason: 'user' })).toEqual({ decision: 'allow' })

  await page.getByRole('button', { name: /Files/ }).first().click()
  await expect.poll(() => sent(page, 'app.open')).toEqual([{ app: 'files', path: '/?path=%2Fsrv%2Fpublish' }])
  expect(context.pages()).toHaveLength(1)
})

test('the client’s Back goes through an app’s own pages first', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await inClient(page)
  await page.goto('/app/users-agents/self')
  await expect.poll(() => sent(page, 'header.set')).toContainEqual({ title: 'Users & Agents', back: true })
  expect(await ask(page, 'frame.back', {})).toEqual({ handled: true })
  await expect(page).toHaveURL(/\/app\/users-agents$/)
  expect(await ask(page, 'frame.back', {})).toEqual({ handled: false })
})

test('in the client a file of the unlocked computer opens in the Files page, which holds the sudo session', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await inClient(page)
  await page.goto('/app/files?fbHost=1')
  const files = page.getByTestId('standalone-files')
  await files.getByRole('button', { name: 'Places' }).click()
  const places = files.getByTestId('files-places')
  await places.getByRole('button', { name: 'Devices' }).click()
  await places.getByTestId('sidebar-unlock-host').click()
  await page.getByRole('textbox', { name: 'Password' }).fill('secret')
  await page.keyboard.press('Enter')
  await files.locator('[data-item-key]').filter({ hasText: 'tmp' }).first().click()
  await files.locator('[data-item-key]').filter({ hasText: 'a b#1.txt' }).first().click()
  const preview = page.getByTestId('standalone-in-page-preview')
  await expect(preview.getByText('mock content of /tmp/a b#1.txt')).toBeVisible()
  expect(await sent(page, 'app.open')).toEqual([])
  await expect(page).toHaveURL(/\/app\/files/)
  expect(await ask(page, 'frame.back', {})).toEqual({ handled: true })
  await expect(preview).toHaveCount(0)
  await expect(files.locator('[data-item-key]').filter({ hasText: 'a b#1.txt' })).toBeVisible()
})

test('Settings keeps the account’s appearance in the client and closing asks the client', async ({ page }) => {
  await inClient(page, { env: { container: 'desktop-app', formFactor: 'desktop', platform: 'linux' }, init: { container: 'desktop-app', formFactor: 'desktop' } })
  await page.goto('/app/settings/appearance')
  await expect(page.getByTestId('standalone-settings')).toHaveAttribute('data-container', 'desktop-app')
  await expect.poll(() => sent(page, 'window.setTitle')).toContainEqual({ title: 'Settings', dirty: false })
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light')
  await page.goto(`/app/preview?open=${encodeURIComponent('cyfs:///home/Documents/Kyoto Trip Plan.md')}`)
  await expect.poll(() => sent(page, 'window.setTitle')).toContainEqual({ title: 'Kyoto Trip Plan.md', dirty: false })
  await page.keyboard.press('Escape')
  await expect.poll(() => sent(page, 'window.requestClose')).toHaveLength(1)
})

test('in a tab of the client MessageHub and HomeStation leave out their own navigation', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await inClient(page, { env: { presentation: 'tab' }, init: { presentation: 'tab' } })
  await page.goto('/homestation')
  await expect(page.getByTestId('standalone-homestation')).toHaveAttribute('data-presentation', 'tab')
  await expect(page.getByTestId('hs-tab-header')).toBeVisible()
  await expect(page.getByTestId('hs-tabbar')).toHaveCount(0)
  await page.getByTestId('hs-avatar').click()
  await expect(page.getByTestId('hs-me')).toBeVisible()
  await expect.poll(() => sent(page, 'header.set')).toContainEqual({ title: 'HomeStation', back: true })
  expect(await ask(page, 'frame.back', {})).toEqual({ handled: true })
  await expect(page.getByTestId('hs-tab-header')).toBeVisible()
  await page.getByTestId('hs-publish-open').click()
  await expect(page.getByTestId('homestation')).toContainText('New post')

  await page.goto('/messagehub')
  await expect(page.getByTestId('standalone-messagehub')).toHaveAttribute('data-presentation', 'tab')
  await page.getByTestId('hub-more-menu').click()
  await expect(page.getByTestId('media-settings')).toBeVisible()
  await expect(page.getByTestId('messagehub-home')).toHaveCount(0)
  await page.goto('/messagehub?entityId=did%3Abuckyos%3Aagent%3Acodeassistant')
  await expect(page.locator('textarea')).toBeVisible()
  await expect.poll(() => sent(page, 'header.set')).toContainEqual({ title: 'MessageHub', back: true })
  expect(await ask(page, 'frame.back', {})).toEqual({ handled: true })
  await expect(page.getByTestId('hub-more-menu')).toBeVisible()
})
