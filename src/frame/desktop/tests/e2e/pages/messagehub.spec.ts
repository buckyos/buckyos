import { expect, test, type Page } from '@playwright/test'

async function getComposerMetrics(page: Page) {
  return page.evaluate(() => {
    const textarea = document.querySelector('textarea')
    const composer = document.querySelector('[data-testid="message-composer"]')
    if (
      !(textarea instanceof HTMLTextAreaElement)
      || !(composer instanceof HTMLElement)
    ) {
      return null
    }

    return {
      textareaHeight: Math.round(textarea.getBoundingClientRect().height),
      composerHeight: Math.round(composer.getBoundingClientRect().height),
    }
  })
}

test('messagehub composer auto-resizes for multiline text and attachments', async ({
  page,
}) => {
  await page.goto('/messagehub')

  const textarea = page.locator('textarea')
  await expect(textarea).toBeVisible()

  const initial = await getComposerMetrics(page)
  expect(initial).not.toBeNull()

  await textarea.click()
  await page.keyboard.type('line 1')
  await page.keyboard.press('Shift+Enter')
  await page.keyboard.type('line 2')
  await page.keyboard.press('Shift+Enter')
  await page.keyboard.type('line 3')

  await expect.poll(async () => (await getComposerMetrics(page))?.composerHeight ?? 0)
    .toBeGreaterThan(initial?.composerHeight ?? 0)
  const multiline = await getComposerMetrics(page)
  expect(multiline?.textareaHeight).toBeGreaterThan(initial?.textareaHeight ?? 0)

  await page.locator('input[type="file"]').nth(0).setInputFiles([
    'package.json',
  ])

  await expect.poll(async () => (await getComposerMetrics(page))?.composerHeight ?? 0)
    .toBeGreaterThan(multiline?.composerHeight ?? 0)

  await page.getByRole('button', { name: /Clear|清空/ }).click()

  await expect.poll(async () => (await getComposerMetrics(page))?.composerHeight ?? 0)
    .toBe(multiline?.composerHeight)
})

for (const viewport of [{ width: 1440, height: 900 }, { width: 375, height: 812 }]) {
  test(`codeassistant wheel scrolling preserves message positions at ${viewport.width}px`, async ({
    page,
  }) => {
    const errors: string[] = []
    page.on('pageerror', (error) => errors.push(error.message))
    page.on('console', (message) => {
      if (message.type() === 'error') errors.push(message.text())
    })
    await page.addInitScript(() => {
      window.addEventListener('error', (event) => console.error(event.message))
    })

    await page.setViewportSize(viewport)
    await page.route('https://upload.wikimedia.org/**', (route) => route.fulfill({
      contentType: 'image/svg+xml',
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"><rect width="800" height="600" fill="lightblue"/></svg>',
    }))
    await page.goto('/messagehub')

    const history = page.locator('.shell-scrollbar').filter({ has: page.locator('[data-index]') })
    await expect.poll(async () => Number(
      await history.locator('[data-index]').last().getAttribute('data-index'),
    )).toBeGreaterThan(2000)
    await expect.poll(() => history.evaluate((element) => (
      element.scrollHeight - element.clientHeight - element.scrollTop
    ))).toBeLessThanOrEqual(1)
    await expect.poll(() => history.locator('img').evaluateAll((images) => (
      images.every((image) => image instanceof HTMLImageElement && image.complete)
    ))).toBe(true)
    await history.hover()

    for (const delta of [-220, 220]) {
      for (let step = 0; step < 40; step += 1) {
        const anchor = await history.evaluate((element, delta) => {
          const viewport = element.getBoundingClientRect()
          const row = Array.from(element.querySelectorAll<HTMLElement>('[data-index]'))
            .find((item) => item.getBoundingClientRect().bottom > viewport.top + viewport.height / 2)!

          return {
            index: row.dataset.index,
            top: row.getBoundingClientRect().top,
            delta: delta < 0
              ? Math.max(delta, -element.scrollTop)
              : Math.min(delta, element.scrollHeight - element.clientHeight - element.scrollTop),
          }
        }, delta)

        await page.mouse.wheel(0, delta)
        await page.waitForTimeout(100)

        const top = await history.locator(`[data-index="${anchor.index}"]`)
          .evaluate((element) => element.getBoundingClientRect().top)
        expect(Math.abs(top - anchor.top + anchor.delta), `wheel ${delta}, step ${step}`)
          .toBeLessThanOrEqual(1)
      }
    }

    await history.hover()
    await page.mouse.wheel(0, -440)
    const bottomButton = page.getByRole('button', { name: 'Scroll to bottom' })
    await expect(bottomButton).toBeVisible()
    await bottomButton.click()
    await expect.poll(() => history.evaluate((element) => (
      element.scrollHeight - element.clientHeight - element.scrollTop
    ))).toBeLessThanOrEqual(1)

    await history.hover()
    await page.mouse.wheel(0, -220)
    await page.waitForTimeout(600)
    await expect(bottomButton).toBeVisible()
    expect(await history.evaluate((element) => (
      element.scrollHeight - element.clientHeight - element.scrollTop
    ))).toBeGreaterThan(200)

    await page.screenshot({ path: `test-results/messagehub-scroll-${viewport.width}.png` })
    expect(errors).toEqual([])
  })
}

declare global {
  interface Window { __messageHubMock: import('../../../src/app/messagehub/mock/store').MessageHubMockStore }
}
const SELF = 'did:buckyos:user:self'
const CODER = 'did:buckyos:agent:codeassistant'
const ALICE = 'did:buckyos:person:alice'
const OWN = { viewerDid: SELF, ownerDid: SELF, mode: 'self' as const }

async function openHub(page: import('@playwright/test').Page, entityId = CODER) {
  await page.route('https://upload.wikimedia.org/**', route => route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"/>' }))
  await page.goto(`/messagehub?entityId=${encodeURIComponent(entityId)}`)
  await expect(page.getByRole('button', { name: 'Sessions', exact: true })).toBeVisible()
}
async function createSession(page: import('@playwright/test').Page, title: string) {
  await page.getByRole('button', { name: 'New Session', exact: true }).last().click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('Title (optional)', { exact: true }).fill(title)
  await dialog.getByRole('combobox', { name: 'Connection', exact: true }).selectOption('native')
  await dialog.getByRole('button', { name: 'Create', exact: true }).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-raw-count', '0')
  return page.evaluate(({ context, entity }) => window.__messageHubMock.sessions(context, entity)[0].id, { context: OWN, entity: CODER })
}
async function details(page: import('@playwright/test').Page) {
  await page.getByRole('button', { name: 'Session details', exact: true }).last().click()
  await expect(page.getByTestId('session-details')).toBeVisible()
  return page.getByTestId('session-details')
}

test('New Session includes native users and selecting an empty user creates one reusable default', async ({ page }) => {
  await openHub(page)
  await page.getByRole('button', { name: 'New Session', exact: true }).first().click()
  const dialog = page.getByRole('dialog')
  const target = dialog.getByRole('combobox').first()
  await expect(target.locator('option', { hasText: 'Bob Zhang' })).toHaveCount(1)
  await target.selectOption('did:buckyos:person:bob')
  await expect(dialog.getByRole('button', { name: 'Create', exact: true })).toBeEnabled()
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  await page.evaluate(async context => {
    const store = window.__messageHubMock
    for (const session of store.sessions(context, 'did:buckyos:person:bob')) await store.manage(context, session.id, 'delete')
  }, OWN)
  await page.getByRole('button', { name: /^Bob Zhang/ }).click()
  await expect(page.locator('textarea')).toBeVisible()
  await page.locator('textarea').fill('Draft in the default session')
  await page.getByRole('button', { name: /^CodeAssistant/ }).click()
  await page.getByRole('button', { name: /^Bob Zhang/ }).dblclick()
  await expect(page.locator('textarea')).toHaveValue('Draft in the default session')
  expect(await page.evaluate(context => window.__messageHubMock.sessions(context, 'did:buckyos:person:bob').length, OWN)).toBe(1)
})

test('agent entity entry creates a missing default and skips pinned or tunnel sessions', async ({ page }) => {
  await openHub(page, 'did:bns:assistant.alice')
  await expect(page.locator('textarea')).toBeVisible()
  const before = await page.evaluate(context => window.__messageHubMock.sessions(context, 'did:bns:assistant.alice').map(session => session.id), OWN)
  expect(before).toHaveLength(1)
  await page.reload()
  await expect(page.locator('textarea')).toBeVisible()
  expect(await page.evaluate(context => window.__messageHubMock.sessions(context, 'did:bns:assistant.alice').map(session => session.id), OWN)).toEqual(before)
  await page.evaluate(async ({ context, agent }) => {
    const store = window.__messageHubMock
    await store.updatePreferences(context, 'session-coder-2', { pinned: true })
    await store.discoverConnection(context.ownerDid, agent, { kind: 'tunnel', tunnelInstanceId: 'agent-tunnel', endpointDid: 'did:msgtunnel:agent.user.agent-tunnel', connectionName: 'Agent Telegram', connected: true, canSend: true, supportsMultipleSessions: false, canCreateRemoteSession: false })
  }, { context: OWN, agent: CODER })
  await page.getByRole('button', { name: /^CodeAssistant/ }).click()
  await page.getByRole('button', { name: 'Sessions', exact: true }).click()
  await expect(page.locator('[data-session-id="session-coder-1"] button').first()).toHaveAttribute('aria-current', 'true')
  await expect(page.locator('[data-session-id="session-coder-2"] button').first()).not.toHaveAttribute('aria-current', 'true')
  await page.evaluate(async ({ context, agent }) => {
    const store = window.__messageHubMock
    for (const session of store.sessions(context, agent, 'active')) if (session.binding.kind === 'native') await store.manage(context, session.id, 'archive')
  }, { context: OWN, agent: CODER })
  await page.getByRole('button', { name: /^Bucky Assistant/ }).click()
  await page.getByRole('button', { name: /^CodeAssistant/ }).click()
  await expect(page.locator('textarea')).toBeVisible()
  expect(await page.evaluate(({ context, agent }) => window.__messageHubMock.sessions(context, agent, 'active').filter(session => session.binding.kind === 'native').length, { context: OWN, agent: CODER })).toBe(1)
})

test('default creation can retry and a delayed completion cannot replace a newer entity selection', async ({ page }) => {
  await openHub(page)
  await page.evaluate(() => window.__messageHubMock.configure({ failNext: 'mock_failure' }))
  await page.getByRole('button', { name: /^Bucky Assistant/ }).click()
  await expect(page.getByRole('alert')).toBeVisible()
  await page.getByRole('button', { name: 'Retry', exact: true }).click()
  await expect(page.locator('textarea')).toBeVisible()
  await page.evaluate(async context => {
    const store = window.__messageHubMock
    for (const session of store.sessions(context, 'did:bns:assistant.alice')) await store.manage(context, session.id, 'delete')
    store.configure({ delayMs: 800 })
  }, OWN)
  await page.getByRole('button', { name: /^Bucky Assistant/ }).dblclick()
  await page.getByRole('button', { name: /^CodeAssistant/ }).click()
  await expect.poll(() => page.evaluate(context => window.__messageHubMock.sessions(context, 'did:bns:assistant.alice').length, OWN)).toBe(1)
  await expect(page.getByRole('button', { name: 'Entity details: CodeAssistant' })).toBeVisible()
})

for (const width of [1440, 375]) {
  test(`session lifecycle, independent drafts and details at ${width}px`, async ({ page }) => {
    const errors: string[] = []
    page.on('pageerror', error => errors.push(error.message))
    await page.setViewportSize({ width, height: 900 })
    await openHub(page)
    await page.getByRole('button', { name: 'New Session', exact: true }).last().click()
    await page.screenshot({ path: `test-results/messagehub-create-${width}.png` })
    await page.getByRole('dialog').getByRole('button', { name: 'Cancel' }).click()
    const first = await createSession(page, 'Release review')
    await expect(page.getByText('Start a conversation', { exact: true })).toBeVisible()
    await page.locator('textarea').fill('First message in its own session')
    await page.locator('textarea').press('Enter')
    await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-raw-count', '1')
    await expect(page.locator('textarea')).toHaveValue('')
    await page.reload()
    await expect(page.getByTestId('conversation-history').getByText('First message in its own session', { exact: true })).toBeVisible()
    const second = await createSession(page, 'Release review')
    expect(second).not.toBe(first)
    await page.locator('textarea').fill('Keep this draft')
    await page.locator('input[type="file"]').first().setInputFiles('package.json')
    await expect.poll(() => page.evaluate(({ context, id }) => window.__messageHubMock.attachments(context, id).length, { context: OWN, id: second })).toBe(1)
    const panel = await details(page)
    await panel.getByLabel('Shared title', { exact: true }).fill('Shared release')
    await panel.locator('form').filter({ has: page.getByLabel('Shared title', { exact: true }) }).getByRole('button', { name: 'Save', exact: true }).click()
    await expect(panel.getByRole('status')).toContainText('Saved')
    await panel.getByLabel('My session nickname').fill('Release captain')
    await panel.locator('form').filter({ has: page.getByLabel('My session nickname') }).getByRole('button', { name: 'Save', exact: true }).click()
    await panel.getByLabel('Personal display title').fill('My release')
    await panel.locator('form').filter({ has: page.getByLabel('Personal display title') }).getByRole('button', { name: 'Save', exact: true }).click()
    await expect(panel.getByRole('heading', { name: 'My release', exact: true })).toBeVisible()
    await page.screenshot({ path: `test-results/messagehub-details-${width}.png` })
    await panel.getByRole('button', { name: 'Close', exact: true }).click()
    await expect(page.locator('textarea')).toHaveValue('Keep this draft')
    await expect(page.getByTitle('package.json', { exact: true })).toBeVisible()
    await expect(page.getByTestId('action-message')).toHaveCount(2)
    await page.screenshot({ path: `test-results/messagehub-actions-${width}.png` })
    await page.getByLabel('Show Action Messages').uncheck()
    await expect(page.getByText('Current messages are filtered', { exact: true })).toBeVisible()
    await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-raw-count', '2')
    await page.getByLabel('Show Action Messages').check()
    await expect(page.getByTestId('action-message')).toHaveCount(2)
    await page.getByRole('button', { name: 'Sessions', exact: true }).click()
    const row = page.locator(`[data-session-id="${second}"]`)
    await row.hover()
    await page.screenshot({ path: `test-results/messagehub-sessions-${width}.png` })
    await row.getByRole('button', { name: 'Manage session: My release' }).click()
    await page.screenshot({ path: `test-results/messagehub-manage-${width}.png` })
    await page.getByRole('dialog').getByRole('button', { name: 'Cancel' }).click()
    await expect(row.getByRole('button', { name: 'Manage session: My release' })).toBeFocused()
    await row.getByRole('button', { name: 'Manage session: My release' }).click()
    await page.getByRole('dialog').getByRole('button', { name: 'Archive', exact: true }).click()
    await expect(row).toHaveCount(0)
    await page.getByTestId('session-sidebar').getByRole('button', { name: /Archived/ }).click()
    await expect(row).toBeVisible()
    await row.getByRole('button').first().click()
    const archivedPanel = await details(page)
    await expect(archivedPanel.getByText('Archived', { exact: true })).toBeVisible()
    await archivedPanel.getByRole('button', { name: 'Manage session', exact: true }).click()
    await page.getByRole('dialog').getByRole('button', { name: 'Restore', exact: true }).click()
    if (width === 375) await page.getByRole('button', { name: 'Sessions', exact: true }).click()
    await row.getByRole('button').first().click()
    await expect(page.locator('textarea')).toHaveValue('Keep this draft')
    const deletePanel = await details(page)
    await deletePanel.getByRole('button', { name: 'Manage session', exact: true }).click()
    await page.getByRole('dialog').getByRole('button', { name: 'Delete permanently', exact: true }).click()
    await expect(page.getByRole('dialog')).toHaveCount(0)
    await page.reload()
    await expect(page.getByRole('button', { name: 'Sessions', exact: true })).toBeVisible()
    expect(await page.evaluate(({ context, id }) => window.__messageHubMock.sessions(context).some(session => session.id === id), { context: OWN, id: second })).toBe(false)
    expect(await page.evaluate(({ context, id }) => window.__messageHubMock.draft(context, id), { context: OWN, id: second })).toBe('')
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
    expect(errors).toEqual([])
  })
}

test('status, Action logs, clock ticks and delivery changes do not reorder sessions', async ({ page }) => {
  await openHub(page)
  await page.getByRole('button', { name: 'Sessions', exact: true }).click()
  const before = await page.getByTestId('session-row').evaluateAll(rows => rows.map(row => row.getAttribute('data-session-id')))
  const old = page.locator('[data-session-id="session-coder-2"]')
  await expect(old.locator('time')).toHaveText('4h')
  await old.getByRole('button').first().click()
  const original = await page.evaluate(({ context }) => {
    const store = window.__messageHubMock, sessions = store.sessions(context)
    const session = sessions.find(item => item.id === 'session-coder-2')!
    store.configure({ now: store.now() })
    store.injectRuntime(context.ownerDid, session.id, { memberDid: session.entityId, status: 'typing', expiresAt: store.now() + 5000 })
    return { time: session.lastActiveAt, count: store.reader(context, session.id).totalCount }
  }, { context: OWN })
  await expect(page.getByTestId('session-runtime')).toContainText('typing…')
  await page.evaluate(async context => { await window.__messageHubMock.updateState(context, 'session-coder-2', 'shared', { title: 'Changed title', description: '' }) }, OWN)
  await expect(old.getByRole('button').first()).toContainText('Changed title')
  await expect(old.locator('time')).toHaveText('4h')
  expect(await page.getByTestId('session-row').evaluateAll(rows => rows.map(row => row.getAttribute('data-session-id')))).toEqual(before)
  await page.evaluate(async context => {
    const store = window.__messageHubMock, reader = store.reader(context, 'session-coder-2')
    const [message] = await reader.readRange(0, 1)
    await store.injectDelivery(context.ownerDid, 'session-coder-2', String(message.ui_message_id), 'read')
    await store.injectMessage(context.ownerDid, 'session-coder-2', { ...message, created_at_ms: 1, ui_message_id: 'late-message', content: { content: 'Late arrival' } })
  }, OWN)
  await page.evaluate(() => { const store = window.__messageHubMock; store.configure({ now: store.now() + 60_000 }) })
  await expect(page.getByTestId('session-runtime')).toBeEmpty()
  expect(await page.evaluate(context => window.__messageHubMock.sessions(context).find(session => session.id === 'session-coder-2')?.lastActiveAt, OWN)).toBe(original.time)
  await page.evaluate(async context => {
    const store = window.__messageHubMock
    await store.injectMessage(context.ownerDid, 'session-coder-2', { from: 'did:buckyos:agent:codeassistant', to: [context.ownerDid], kind: 'chat', created_at_ms: store.now(), ui_message_id: 'new-ordinary', content: { content: 'New ordinary message' } })
  }, OWN)
  await expect(page.getByTestId('session-row').first()).toHaveAttribute('data-session-id', 'session-coder-2')
  await expect(old.locator('time')).toHaveText('now')
})

test('failed creation and deletion retain input, selection and data, and block duplicate submits', async ({ page }) => {
  await openHub(page)
  await page.getByRole('button', { name: 'New Session', exact: true }).last().click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('Title (optional)').fill('Retryable')
  await dialog.getByRole('combobox', { name: 'Connection', exact: true }).selectOption('native')
  await page.evaluate(() => window.__messageHubMock.configure({ failNext: 'mock_failure', delayMs: 100 }))
  await dialog.getByRole('button', { name: 'Create', exact: true }).click()
  await expect(dialog.getByRole('alert')).toBeVisible()
  await expect(dialog.getByLabel('Title (optional)')).toHaveValue('Retryable')
  await dialog.getByRole('button', { name: 'Create', exact: true }).dblclick()
  await expect(dialog).toHaveCount(0)
  expect(await page.evaluate(context => window.__messageHubMock.sessions(context).filter(session => session.shared.title === 'Retryable').length, OWN)).toBe(1)
  const panel = await details(page)
  await panel.getByRole('button', { name: 'Manage session', exact: true }).click()
  await page.evaluate(() => window.__messageHubMock.configure({ failNext: 'mock_failure' }))
  await dialog.getByRole('button', { name: 'Delete permanently' }).click()
  await expect(dialog.getByRole('alert')).toBeVisible()
  await expect(panel.getByRole('heading', { name: 'Retryable' })).toBeVisible()
  await dialog.getByRole('button', { name: 'Delete permanently' }).click()
  await expect(dialog).toHaveCount(0)
})

test('tunnel writing, explicit creation policy, connection limits and binding isolation', async ({ page }) => {
  await openHub(page, ALICE)
  await page.getByRole('button', { name: 'New Session', exact: true }).last().click()
  await page.getByRole('dialog').getByRole('combobox', { name: 'Connection', exact: true }).selectOption('telegram-work')
  await expect(page.getByRole('dialog').getByRole('button', { name: 'Create', exact: true })).toBeDisabled()
  await page.getByRole('dialog').getByRole('button', { name: 'Cancel' }).click()
  await page.getByRole('button', { name: 'Entity details: Alice Chen' }).click()
  await page.getByLabel('Allow manual sessions with this entity').selectOption('allow')
  await page.getByRole('button', { name: 'Save', exact: true }).click()
  await page.getByRole('button', { name: 'Close', exact: true }).last().click()
  await page.getByRole('button', { name: 'New Session', exact: true }).last().click()
  const dialog = page.getByRole('dialog')
  await expect(dialog.getByRole('button', { name: 'Create', exact: true })).toBeDisabled()
  await dialog.getByRole('combobox', { name: 'Connection', exact: true }).selectOption('telegram-personal')
  await expect(dialog.getByRole('button', { name: 'Create', exact: true })).toBeDisabled()
  await dialog.getByRole('combobox', { name: 'Connection', exact: true }).selectOption('telegram-work')
  await dialog.getByLabel('Title (optional)').fill('External review')
  await dialog.getByRole('button', { name: 'Create', exact: true }).click()
  await expect(page.locator('textarea')).toHaveCount(0)
  const panel = await details(page)
  await expect(panel.getByLabel('Shared title', { exact: true })).toBeDisabled()
  await panel.getByRole('button', { name: 'Enable writing' }).click()
  await expect(dialog).toContainText('incorrect or inconsistent')
  await dialog.getByRole('button', { name: 'Enable writing' }).click()
  await expect(panel.getByLabel('Shared title', { exact: true })).toBeDisabled()
  await panel.getByRole('button', { name: 'Close', exact: true }).click()
  await page.locator('textarea').fill('Target this connection only')
  await page.locator('textarea').press('Enter')
  await expect(page.getByTestId('conversation-history').getByText('Target this connection only', { exact: true })).toBeVisible()
  const sent = await page.evaluate(async ({ context, entity }) => {
    const store = window.__messageHubMock, session = store.sessions(context, entity)[0]
    return { session, messages: await store.reader(context, session.id).readRange(0, 10) }
  }, { context: OWN, entity: ALICE })
  expect(sent.messages[0].to).toEqual(['did:telegram:telegram-work:alice'])
  expect(sent.messages[0].ui_session_id).toBe(sent.session.id)
  await page.reload()
  await expect(page.getByRole('button', { name: 'Sessions', exact: true })).toBeVisible()
  await expect(page.locator('textarea')).toHaveCount(0)
})

test('archived states stay archived; deleted tunnel history cannot be replayed', async ({ page }) => {
  await openHub(page, ALICE)
  const result = await page.evaluate(async context => {
    const store = window.__messageHubMock, id = 'telegram-work-general'
    const message = { kind: 'chat' as const, from: 'did:telegram:telegram-work:alice', to: [context.ownerDid], created_at_ms: store.now(), ui_message_id: 'prior-history', content: { content: 'Prior history' } }
    await store.injectMessage(context.ownerDid, id, message)
    await store.manage(context, id, 'archive')
    store.injectRuntime(context.ownerDid, id, { memberDid: 'did:buckyos:person:alice', status: 'typing', expiresAt: store.now() + 10000 })
    await store.injectMessage(context.ownerDid, id, { ...message, kind: 'event', ui_message_id: 'log', content: { content: 'Updated title', machine: { intent: 'buckyos.action_log', data: { schema_version: 1, action: 'session.title_changed' } } } })
    const archived = store.sessions(context).find(session => session.id === id)!.lifecycle
    await store.injectMessage(context.ownerDid, id, { ...message, ui_message_id: 'next', created_at_ms: store.now() + 1 })
    const restored = store.sessions(context).find(session => session.id === id)!.lifecycle
    store.configure({ now: store.now() + 100 })
    await store.manage(context, id, 'delete')
    await store.injectMessage(context.ownerDid, id, message)
    const replayed = store.sessions(context).some(session => session.id === id)
    await store.injectMessage(context.ownerDid, id, { ...message, created_at_ms: store.now() + 1, ui_message_id: 'after-deletion', content: { content: 'Fresh start' } })
    const messages = await store.reader(context, id).readRange(0, 100)
    return { archived, restored, replayed, messages }
  }, OWN)
  expect(result.archived).toBe('archived')
  expect(result.restored).toBe('active')
  expect(result.replayed).toBe(false)
  expect(result.messages.map(message => message.content.content)).toEqual(['Fresh start'])
  await page.reload()
  await expect(page.getByTestId('conversation-history').getByText('Fresh start', { exact: true })).toBeVisible()
})

for (const width of [1440, 375]) {
  test(`Agent owner is isolated and read only at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 })
    await openHub(page)
    await page.evaluate(context => window.__messageHubMock.saveDraft(context, 'session-coder-1', 'Private user draft'), OWN)
    await page.goto(`/messagehub?entityId=${encodeURIComponent(ALICE)}&ownerDid=${encodeURIComponent(CODER)}&mode=observe`)
    await expect(page.getByTestId('owner-banner')).toContainText('Read only')
    await expect(page.locator('textarea')).toHaveCount(0)
    await expect(page.getByRole('button', { name: 'New Session', exact: true }).last()).toBeDisabled()
    const panel = await details(page)
    await expect(panel.getByLabel('Shared title', { exact: true })).toBeDisabled()
    await expect(panel.getByRole('button', { name: 'Manage session', exact: true })).toBeDisabled()
    await page.screenshot({ path: `test-results/messagehub-observer-${width}.png` })
    await panel.getByRole('button', { name: 'Close', exact: true }).click()
    await page.getByLabel('Show Action Messages').uncheck()
    await expect(page.getByLabel('Show Action Messages')).toBeEnabled()
    const state = await page.evaluate(({ context, agent }) => {
      const store = window.__messageHubMock
      return { own: store.preferences(context, 'session-coder-1').showActions, agent: store.preferences({ ...context, ownerDid: agent, mode: 'observe' }, 'session-coder-1').showActions, draft: store.draft(context, 'session-coder-1') }
    }, { context: OWN, agent: CODER })
    expect(state).toEqual({ own: true, agent: false, draft: 'Private user draft' })
    const blocked = await page.evaluate(async ({ context, agent }) => {
      try { await window.__messageHubMock.manage({ ...context, ownerDid: agent, mode: 'observe' }, 'session-coder-1', 'delete'); return false } catch { return true }
    }, { context: OWN, agent: CODER })
    expect(blocked).toBe(true)
    await page.goto('/messagehub?ownerDid=did:buckyos:agent:denied&mode=observe')
    await expect(page.getByRole('alert')).toContainText('permission')
    await expect(page.getByTestId('conversation-history')).toHaveCount(0)
  })
}

test('last session deletion leaves entity intact; non-selected management preserves the draft', async ({ page }) => {
  await openHub(page, 'did:buckyos:person:bob')
  await page.locator('textarea').fill('Bob draft')
  const panel = await details(page)
  await panel.getByRole('button', { name: 'Manage session', exact: true }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Delete permanently' }).click()
  await expect(page.getByTestId('session-details')).toHaveCount(0)
  await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-raw-count', '0')
  await expect(page.getByRole('button', { name: 'Entity details: Bob Zhang' })).toBeVisible()
  await page.getByRole('button', { name: 'Sessions', exact: true }).click()
  await expect(page.getByTestId('session-sidebar').getByRole('button', { name: /Archived/ })).toBeVisible()
  await expect(page.getByTestId('session-row')).toHaveCount(0)
  await openHub(page)
  await page.locator('textarea').fill('Leave current draft alone')
  await page.getByRole('button', { name: 'Sessions', exact: true }).click()
  const other = page.locator('[data-session-id="session-coder-2"]')
  await other.getByRole('button').last().focus()
  await expect(other.getByRole('button').last()).toHaveCSS('opacity', '1')
  expect(await page.locator('button button').count()).toBe(0)
  await other.getByRole('button').last().press('Enter')
  await page.getByRole('dialog').getByRole('button', { name: 'Archive', exact: true }).click()
  await expect(page.locator('textarea')).toHaveValue('Leave current draft alone')
  await expect(page.locator('[data-session-id="session-coder-1"] button').first()).toHaveAttribute('aria-current', 'true')
})

test('cancelled delayed creation does not change a newer selection and refresh retains registered session', async ({ page }) => {
  await openHub(page)
  await page.getByRole('button', { name: 'New Session', exact: true }).last().click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('Title (optional)').fill('Delayed session')
  await dialog.getByRole('combobox', { name: 'Connection', exact: true }).selectOption('native')
  await page.evaluate(() => window.__messageHubMock.configure({ delayMs: 1200 }))
  await dialog.getByRole('button', { name: 'Create', exact: true }).click()
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  await page.getByRole('button', { name: /^Alice Chen/ }).click()
  await expect.poll(() => page.evaluate(context => window.__messageHubMock.sessions(context).some(session => session.shared.title === 'Delayed session'), OWN)).toBe(true)
  await expect(page.getByRole('button', { name: 'Entity details: Alice Chen' })).toBeVisible()
  await page.reload()
  await expect(page.getByRole('button', { name: 'Sessions', exact: true })).toBeVisible()
  expect(await page.evaluate(context => window.__messageHubMock.sessions(context).filter(session => session.shared.title === 'Delayed session').length, OWN)).toBe(1)
})

test('draft attachments survive refresh and failed send retries once in the same session', async ({ page }) => {
  await openHub(page)
  const id = await createSession(page, 'Persistent draft')
  await page.locator('textarea').fill('Message with attachment')
  await page.locator('input[type="file"]').first().setInputFiles('package.json')
  await expect.poll(() => page.evaluate(({ context, id }) => window.__messageHubMock.attachments(context, id).length, { context: OWN, id })).toBe(1)
  await page.reload()
  await expect(page.locator('textarea')).toHaveValue('Message with attachment')
  await expect(page.getByTitle('package.json', { exact: true })).toBeVisible()
  await page.evaluate(() => window.__messageHubMock.configure({ failNext: 'send_failure' }))
  await page.locator('textarea').press('Enter')
  await expect(page.getByTestId('message-composer').getByRole('alert')).toBeVisible()
  await expect(page.locator('textarea')).toHaveValue('Message with attachment')
  await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-raw-count', '0')
  await page.locator('textarea').press('Enter')
  await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-raw-count', '1')
  await expect(page.locator('textarea')).toHaveValue('')
  const reader = await page.evaluate(async ({ context, id }) => window.__messageHubMock.reader(context, id).readRange(0, 5), { context: OWN, id })
  expect(reader).toHaveLength(1)
  expect(reader[0].ui_session_id).toBe(id)
})

for (const source of ['object', 'http'] as const) {
  test(`attachment-only messages render without a caption (${source})`, async ({ page }) => {
    const errors: string[] = []
    page.on('pageerror', error => errors.push(error.message))
    page.on('console', message => {
      if (message.type() === 'error' && message.text().includes('ErrorBoundary')) errors.push(message.text())
    })
    await page.route('https://upload.wikimedia.org/**', route => route.fulfill({
      contentType: 'image/svg+xml',
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="80" height="60"><rect width="80" height="60" fill="lightblue"/></svg>',
    }))
    await openHub(page)
    const id = await createSession(page, 'Photo without caption')
    if (source === 'object') {
      await page.evaluate(async () => {
        const modulePath = '/src/app/messagehub/conversation/history/objectAccess.ts'
        const { registerObjectAccess } = await import(modulePath)
        registerObjectAccess({
          describe: async (objId: string) => ({ objId, name: 'photo.png', mimeType: 'image/png', size: 100, isFile: true }),
          contentUrl: async () => 'https://upload.wikimedia.org/photo.png',
        })
      })
    }
    await page.evaluate(async ({ context, id, source }) => {
      await window.__messageHubMock.injectMessage(context.ownerDid, id, {
        kind: 'chat', from: context.ownerDid, to: ['did:buckyos:agent:codeassistant'],
        created_at_ms: window.__messageHubMock.now(), ui_message_id: 'photo-no-caption',
        content: { format: 'text/plain', refs: [{ role: 'input', label: 'photo.png', target: { type: 'data_obj', obj_id: 'cyfile:photo', uri_hint: source === 'object' ? 'cyfs://cyfile:photo' : 'https://upload.wikimedia.org/photo.png' } }] },
      })
    }, { context: OWN, id, source })
    const history = page.getByTestId('conversation-history')
    await expect(history.getByRole('img', { name: 'photo.png', exact: true })).toBeVisible()
    if (source === 'object') await expect(history.getByTestId('attachment-ready')).toBeVisible()
    if (source === 'http') {
      await page.reload()
      await expect(history.getByRole('img', { name: 'photo.png', exact: true })).toBeVisible()
      await expect(page.locator('textarea')).toBeVisible()
      await expect(history).toHaveAttribute('data-raw-count', '1')
    }
    expect(errors).toEqual([])
  })
}

test('unknown Action schemas and actors are safe; ordinary events remain when filtering', async ({ page }) => {
  await openHub(page)
  const id = await createSession(page, 'Action edge cases')
  await page.evaluate(async ({ context, id }) => {
    const store = window.__messageHubMock
    for (const [messageId, content, machine] of [
      ['event-plain', 'An ordinary event', undefined],
      ['action-remove', 'Removal', { intent: 'buckyos.action_log', data: { schema_version: 1, action: 'entity.member_removed', actor_did: 'Alice', subject_did: 'Bob' } }],
      ['action-unknown', 'Future event summary', { intent: 'buckyos.action_log', data: { schema_version: 99, action: 'execute_shell', payload: 'do not execute' } }],
      ['action-missing', 'Member removed', { intent: 'buckyos.action_log', data: { schema_version: 1, action: 'entity.member_removed', subject_did: 'Carol' } }],
    ] as const) await store.injectMessage(context.ownerDid, id, { kind: 'event', from: context.ownerDid, to: [], created_at_ms: store.now(), ui_message_id: messageId, content: { content, machine } })
  }, { context: OWN, id })
  await expect(page.getByTestId('action-message')).toHaveCount(3)
  await expect(page.getByTestId('action-message').filter({ hasText: 'Alice removed Bob' })).toBeVisible()
  await expect(page.getByTestId('action-message').filter({ hasText: 'Unknown actor removed Carol' })).toBeVisible()
  await expect(page.getByTestId('action-message').filter({ hasText: 'Unsupported event version' })).toBeVisible()
  await page.getByLabel('Show Action Messages').uncheck()
  await expect(page.getByTestId('action-message')).toHaveCount(0)
  await expect(page.getByTestId('conversation-history').getByText('An ordinary event')).toBeVisible()
  await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-raw-count', '4')
  await page.getByLabel('Show Action Messages').check()
  await expect(page.getByTestId('action-message')).toHaveCount(3)
})

for (const width of [1440, 375]) {
  test(`details and Action filtering preserve a long-history anchor at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 })
    await openHub(page)
    const id = await createSession(page, 'Mixed history')
    await page.evaluate(async ({ context, id }) => {
      const store = window.__messageHubMock, now = store.now()
      for (let index = 0; index < 100; index++) {
        await store.injectMessage(context.ownerDid, id, { kind: index % 2 ? 'event' : 'chat', from: 'did:buckyos:agent:codeassistant', to: [context.ownerDid], created_at_ms: now + index, ui_message_id: `mixed-${index}`, content: { content: index % 2 ? `Action ${index}` : `Content ${index}`, ...(index % 2 ? { machine: { intent: 'buckyos.action_log', data: { schema_version: 1, action: 'session.title_changed' } } } : {}) } })
      }
    }, { context: OWN, id })
    const history = page.getByTestId('conversation-history').locator('.shell-scrollbar')
    await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-raw-count', '100')
    await history.hover()
    await page.mouse.wheel(0, -1200)
    await expect(page.getByRole('button', { name: 'Scroll to bottom' })).toBeVisible()
    const anchor = await history.evaluate(container => {
      const top = container.getBoundingClientRect().top
      const row = [...container.querySelectorAll<HTMLElement>('[data-message-index]')].find(row => Number(row.dataset.messageIndex) % 2 === 0 && row.getBoundingClientRect().bottom > top)!
      return { index: row.dataset.messageIndex!, top: row.getBoundingClientRect().top - top }
    })
    const panel = await details(page)
    await panel.getByRole('button', { name: 'Close', exact: true }).click()
    await expect.poll(() => history.locator(`[data-message-index="${anchor.index}"]`).evaluate((row, top) => Math.abs(row.getBoundingClientRect().top - row.closest('.shell-scrollbar')!.getBoundingClientRect().top - top), anchor.top)).toBeLessThan(2)
    await page.getByLabel('Show Action Messages').uncheck()
    await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-visible-count', '51')
    await expect(history.locator(`[data-message-index="${anchor.index}"]`)).toBeVisible()
    await expect.poll(() => history.locator(`[data-message-index="${anchor.index}"]`).evaluate((row, top) => Math.abs(row.getBoundingClientRect().top - row.closest('.shell-scrollbar')!.getBoundingClientRect().top - top), anchor.top)).toBeLessThan(2)
    await page.getByLabel('Show Action Messages').check()
    await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-visible-count', '101')
  })
}

test('rapid sends are queued in order while typing continues; a failure returns every unsent draft', async ({ page }) => {
  await openHub(page)
  const id = await createSession(page, 'Rapid sends')
  const input = page.locator('textarea')
  const history = page.getByTestId('conversation-history')
  await page.evaluate(() => window.__messageHubMock.configure({ delayMs: 400 }))
  for (const text of ['FAST1', 'FAST2', 'FAST3', 'FAST4', 'FAST5']) {
    await input.fill(text)
    await input.press('Enter')
    await expect(input).toHaveValue('')
  }
  await expect(history).toHaveAttribute('data-raw-count', '5')
  const sent = await page.evaluate(async ({ context, id }) => (await window.__messageHubMock.reader(context, id).readRange(0, 10)).map(message => message.content.content), { context: OWN, id })
  expect(sent).toEqual(['FAST1', 'FAST2', 'FAST3', 'FAST4', 'FAST5'])
  await expect.poll(() => page.evaluate(({ context, id }) => window.__messageHubMock.draft(context, id), { context: OWN, id })).toBe('')

  await page.evaluate(() => window.__messageHubMock.configure({ delayMs: 400, failNext: 'send_failure' }))
  await input.fill('LOST1')
  await input.press('Enter')
  await input.fill('LOST2')
  await input.press('Enter')
  await input.fill('still typing')
  await expect(page.getByTestId('message-composer').getByRole('alert')).toBeVisible()
  await expect(input).toHaveValue('LOST1\nLOST2\nstill typing')
  await expect(history).toHaveAttribute('data-raw-count', '5')
  await page.evaluate(() => window.__messageHubMock.configure({ delayMs: 0 }))
})

test('touch controls have names and 44px targets; message status is readable text', async ({ page }) => {
  await page.setViewportSize({ width: 412, height: 839 })
  await openHub(page)
  await createSession(page, 'Accessible controls')
  const composer = page.getByTestId('message-composer')
  const targets = [
    page.getByRole('button', { name: 'Back to conversation list', exact: true }),
    page.getByRole('button', { name: 'Sessions', exact: true }),
    page.getByRole('button', { name: /^Entity details:/ }),
    page.getByRole('button', { name: 'Session details', exact: true }),
    composer.getByRole('button', { name: 'Add attachment', exact: true }),
    composer.getByRole('button', { name: 'Send', exact: true }),
  ]
  for (const target of targets) {
    const box = await target.boundingBox()
    expect(box?.width ?? 0).toBeGreaterThanOrEqual(44)
    expect(box?.height ?? 0).toBeGreaterThanOrEqual(44)
  }
  await page.locator('input[type="file"]').first().setInputFiles('package.json')
  await expect(composer.getByRole('button', { name: 'Remove package.json', exact: true })).toBeVisible()
  await page.locator('textarea').fill('Status check')
  await page.locator('textarea').press('Enter')
  await expect(page.getByTestId('conversation-history').getByRole('img', { name: 'Sent', exact: true })).toBeVisible()
})

test('attachments over the sendable size are refused before upload', async ({ page }) => {
  await openHub(page)
  await createSession(page, 'Large attachment')
  await page.locator('input[type="file"]').first().setInputFiles({ name: 'big.bin', mimeType: 'application/octet-stream', buffer: Buffer.alloc(32 * 1024 * 1024 + 1) })
  await expect(page.getByTestId('attachment-too-large')).toContainText('big.bin')
  await expect(page.getByTestId('message-composer').getByRole('button', { name: 'Remove big.bin', exact: true })).toHaveCount(0)
  await page.locator('input[type="file"]').first().setInputFiles('package.json')
  await expect(page.getByTestId('attachment-too-large')).toHaveCount(0)
  await expect(page.getByTestId('message-composer').getByRole('button', { name: 'Remove package.json', exact: true })).toBeVisible()
})

test('media that loads late pushes later bubbles down instead of covering them', async ({ page }) => {
  await openHub(page)
  const id = await createSession(page, 'Late media')
  let release!: () => void
  const released = new Promise<void>(resolve => { release = resolve })
  await page.route('https://upload.wikimedia.org/late.png', async route => {
    await released
    await route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300"><rect width="400" height="300" fill="orange"/></svg>' })
  })
  await page.evaluate(async ({ context, id }) => {
    const store = window.__messageHubMock
    const base = store.now()
    await store.injectMessage(context.ownerDid, id, {
      kind: 'chat', from: 'did:buckyos:agent:codeassistant', to: [context.ownerDid], created_at_ms: base, ui_message_id: 'late-photo',
      content: { format: 'text/plain', content: 'photo', refs: [{ role: 'input', label: 'late.png', target: { type: 'data_obj', obj_id: 'cyfile:late', uri_hint: 'https://upload.wikimedia.org/late.png' } }] },
    })
    for (let index = 1; index <= 3; index++) {
      await store.injectMessage(context.ownerDid, id, { kind: 'chat', from: 'did:buckyos:agent:codeassistant', to: [context.ownerDid], created_at_ms: base + index, ui_message_id: `after-${index}`, content: { format: 'text/plain', content: `after ${index}` } })
    }
  }, { context: OWN, id })
  const history = page.getByTestId('conversation-history')
  await expect(history.getByText('after 3', { exact: true })).toBeVisible()
  const overlaps = () => page.evaluate(() => {
    const rows = [...document.querySelectorAll<HTMLElement>('[data-testid="conversation-history"] [data-index]')].sort((a, b) => Number(a.dataset.index) - Number(b.dataset.index))
    let count = 0
    for (let index = 1; index < rows.length; index++) if (rows[index].getBoundingClientRect().top < rows[index - 1].getBoundingClientRect().bottom - 0.5) count++
    return count
  })
  expect(await overlaps()).toBe(0)
  release()
  await expect(history.getByRole('img', { name: 'late.png', exact: true })).toBeVisible()
  await expect.poll(() => page.evaluate(() => (document.querySelector('[data-testid="conversation-history"] img[alt="late.png"]') as HTMLImageElement | null)?.naturalWidth ?? 0)).toBeGreaterThan(0)
  expect(await overlaps()).toBe(0)
})

test('a failed delivery explains itself, keeps technical details folded and can be resent', async ({ page }) => {
  await openHub(page)
  const id = await createSession(page, 'Failed delivery')
  await page.evaluate(async ({ context, id }) => {
    await window.__messageHubMock.injectMessage(context.ownerDid, id, {
      kind: 'chat', from: context.ownerDid, to: ['did:buckyos:agent:codeassistant'], created_at_ms: window.__messageHubMock.now(), ui_message_id: 'dead-1', ui_delivery_status: 'failed',
      ui_record: { recordId: 'dead-1', direction: 'out', boxKind: 'SENT', delivery: { overall: 'failed', per_target: [{ target_did: 'did:buckyos:agent:codeassistant', state: 'DEAD', attempts: 1, last_error: { message: 'local dispatch dropped recipient', retryable: false, duplicate_risk: false } }] } },
      content: { format: 'text/plain', content: 'b1' },
    })
  }, { context: OWN, id })
  const failure = page.getByTestId('delivery-failure')
  await expect(failure).toContainText('You can resend it later')
  await expect(failure.getByText(/local dispatch dropped/)).toBeHidden()
  await expect(page.getByTestId('conversation-history').getByRole('img', { name: 'Not delivered', exact: true })).toBeVisible()
  await failure.getByRole('button', { name: 'Resend', exact: true }).click()
  await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-raw-count', '2')
  const texts = await page.evaluate(async ({ context, id }) => (await window.__messageHubMock.reader(context, id).readRange(0, 5)).map(message => message.content.content), { context: OWN, id })
  expect(texts).toEqual(['b1', 'b1'])
})

test('leaving an observed owner rewrites the address and the standalone route leads home', async ({ page }) => {
  await openHub(page)
  await page.goto(`/messagehub?entityId=${encodeURIComponent(ALICE)}&ownerDid=${encodeURIComponent(CODER)}&mode=observe`)
  await page.getByTestId('owner-banner').getByRole('button', { name: 'Return to my messages', exact: true }).click()
  await expect(page.getByTestId('owner-banner')).toHaveCount(0)
  await expect.poll(() => new URL(page.url()).searchParams.get('mode')).toBeNull()
  await page.reload()
  await expect(page.getByTestId('owner-banner')).toHaveCount(0)
  await expect(page.locator('textarea')).toBeVisible()
  await page.getByTestId('messagehub-home').first().click()
  await expect.poll(() => new URL(page.url()).pathname).toBe('/')
})
