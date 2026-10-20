import { expect, test, type Page } from '@playwright/test'

/** The mock control panel (src/api/control_panel_mock.ts) merges this object over its defaults. */
const MOCK_KEY = 'buckyos.mock.control-panel.v1'
const AVATAR_PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEUlEQVR4nGN4VmGDFTEMLQkAN6BmgaudJikAAAAASUVORK5CYII=', 'base64')
const BOT_TOKEN = '123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw'

async function seedMock(page: Page, state: Record<string, unknown>) {
  await page.addInitScript(([key, value]) => {
    if (!window.sessionStorage.getItem('mock-seeded')) {
      window.localStorage.setItem(key, value)
      window.sessionStorage.setItem('mock-seeded', '1')
    }
  }, [MOCK_KEY, JSON.stringify(state)] as const)
}

async function openGuideWizard(page: Page) {
  await page.getByTestId('desktop-app-agent-guide').click()
  const wizard = page.getByTestId('window-agent-setup')
  await expect(wizard).toBeVisible()
  return wizard
}

async function nextPage(wizard: ReturnType<Page['getByTestId']>) {
  await wizard.getByTestId('agent-setup-next').click()
}

test.describe('Agent setup wizard', () => {
  test.use({ viewport: { width: 1440, height: 900 } })

  test('desktop Jarvis guide: create “小白” and the entry turns into the Agent', async ({ page }) => {
    const consoleErrors: string[] = []
    page.on('console', (message) => { if (message.type() === 'error') consoleErrors.push(message.text()) })
    await page.goto('/?scenario=normal')

    const guide = page.getByTestId('desktop-app-agent-guide')
    await expect(guide).toHaveAttribute('title', 'Jarvis')

    const wizard = await openGuideWizard(page)
    // Step 1a: Jarvis prefill, checked name and DID preview.
    await expect(wizard.getByTestId('agent-setup-name')).toHaveValue('jarvis')
    await expect(wizard.getByTestId('agent-setup-nickname')).toHaveValue('Jarvis')
    await expect(wizard.getByTestId('agent-setup-did')).toHaveText('did:bns:jarvis.alice')
    await wizard.getByTestId('agent-setup-name').fill('xiaobai')
    await expect(wizard.getByTestId('agent-setup-next')).toBeDisabled()
    await expect(wizard.getByTestId('agent-setup-did')).toHaveText('did:bns:xiaobai.alice')
    await wizard.getByTestId('agent-setup-nickname').fill('小白')
    await wizard.getByTestId('agent-setup-avatar-input').setInputFiles({ name: 'avatar.png', mimeType: 'image/png', buffer: AVATAR_PNG })
    await expect(wizard.getByText('Your image, cropped to a square.')).toBeVisible()
    await expect(wizard.getByTestId('agent-setup-bio')).toHaveValue(/personal assistant/)
    await wizard.getByRole('button', { name: 'Role supplement' }).click()
    await wizard.getByTestId('agent-setup-role').fill('Answer in Chinese by default.')
    await nextPage(wizard)

    await expect(wizard.getByTestId('agent-setup-owner')).toContainText('alice')
    await expect(wizard.getByTestId('agent-setup-permission-read-write').getByRole('radio')).toBeChecked()
    await expect(wizard.getByTestId('agent-setup-permission-read-only').getByRole('radio')).not.toBeChecked()
    await expect(wizard.getByTestId('agent-setup-permission')).toContainText('always has read and write access to its own working directory')
    await expect(wizard.getByTestId('agent-setup-sharing')).toContainText('Not supported yet')
    await expect(wizard.getByRole('switch', { name: 'Allow other users to use this Agent' })).toBeDisabled()
    await expect(wizard.getByRole('switch', { name: 'Allow this Agent to join group chats' })).not.toBeChecked()
    await nextPage(wizard)

    // Step 2: Loader, real template list, update policy, summary.
    await expect(wizard.getByRole('combobox', { name: 'Agent Loader' })).toHaveText('OpenDAN')
    await expect(wizard.getByRole('combobox', { name: 'Agent template' })).toHaveText('Jarvis · Built in')
    await expect(wizard.getByTestId('agent-setup-template-info')).toContainText('0.7.0')
    const autoUpdate = wizard.getByRole('checkbox', { name: 'Apply updates of this template automatically' })
    await expect(autoUpdate).toBeChecked()
    await autoUpdate.uncheck()
    await expect(wizard.getByTestId('agent-setup-autoupdate-warning')).toBeVisible()
    await autoUpdate.check()
    await expect(wizard.getByTestId('agent-setup-required-done')).toContainText('talk to “小白”')
    await expect(wizard.getByTestId('agent-setup-required-done')).not.toContainText('has been created')
    await nextPage(wizard)

    // Step 3: no Owner Telegram identity; Lark is not supported; skip.
    await expect(wizard.getByText('Bind an external message channel (optional)')).toBeVisible()
    await expect(wizard.getByTestId('agent-setup-identity-missing')).toBeVisible()
    await expect(wizard.getByTestId('agent-setup-channel-telegram').getByRole('radio')).toBeDisabled()
    await expect(wizard.getByTestId('agent-setup-channel-lark').getByRole('radio')).toBeDisabled()
    await expect(wizard.getByTestId('agent-setup-channel-lark')).toContainText('Not supported yet')
    await wizard.getByTestId('agent-setup-skip-channel').click()

    // Confirmation, then the status panel.
    await expect(wizard.getByTestId('agent-setup-confirm-name')).toHaveText('小白')
    await expect(wizard.getByTestId('agent-setup-confirm-channel')).toContainText('None')
    await expect(wizard.getByTestId('agent-setup-confirm-permission')).toContainText('Read and write (default)')
    await wizard.getByTestId('agent-setup-create').click()
    const status = wizard.getByTestId('agent-setup-status')
    await expect(status).toBeVisible()
    await expect(status).toHaveAttribute('data-state', /provisioning|bound/)
    expect(await page.evaluate((key) => {
      const state = JSON.parse(window.localStorage.getItem(key) ?? '{}')
      return state.agents.find((agent: { name: string }) => agent.name === 'xiaobai')?.permission_mode
    }, MOCK_KEY)).toBe('read_write')

    // Closing during creation does not cancel it; the guide reopens this creation's status.
    await page.getByTestId('window-agent-setup').getByRole('button', { name: 'Close' }).first().click()
    await expect(page.getByTestId('window-agent-setup')).toHaveCount(0)
    await expect(page.getByTestId('desktop-app-badge-agent-guide')).toBeVisible()
    await guide.click()
    await expect(page.getByTestId('agent-setup-status')).toBeVisible()
    await expect(page.getByTestId('agent-setup-status-success')).toContainText('“小白” has been created and is ready to use.', { timeout: 15_000 })
    await page.getByTestId('agent-setup-close').click()
    await expect(page.getByTestId('window-agent-setup')).toHaveCount(0)

    // The entry now shows the Agent's name and avatar, and opens that Agent.
    await expect(guide).toHaveAttribute('title', '小白')
    await expect(guide.locator('img')).toHaveAttribute('src', /^data:image\/jpeg/)
    await expect(page.getByTestId('desktop-app-badge-agent-guide')).toHaveCount(0)
    await guide.click()
    const users = page.getByTestId('window-users-agents')
    await expect(users.getByTestId('agent-detail')).toHaveAttribute('data-agent-id', 'xiaobai.alice')
    await expect(users.getByTestId('agent-profile')).toContainText('小白')
    await expect(users.getByTestId('agent-runtime')).toContainText('Answer in Chinese')
    expect(consoleErrors).toEqual([])
  })

  test('limited users get no guide and a disabled Add Agent', async ({ page }) => {
    await seedMock(page, { account: { user_id: 'lim', user_name: 'Lim', user_type: 'limited', did: 'did:bns:lim' } })
    await page.goto('/?scenario=normal')
    await expect(page.getByTestId('desktop-app-settings')).toBeVisible()
    await expect(page.getByTestId('desktop-app-agent-guide')).toHaveCount(0)
    await page.getByTestId('desktop-app-users-agents').click()
    const users = page.getByTestId('window-users-agents')
    const addAgent = users.getByTestId('users-agents-add-agent')
    await expect(addAgent).toHaveAttribute('aria-disabled', 'true')
    // Disabled for this account, it still explains why when pressed.
    await addAgent.click({ force: true })
    await expect(page.getByRole('alert').filter({ hasText: 'Your account type cannot create Agents.' })).toBeVisible()
    await expect(page.getByTestId('window-agent-setup')).toHaveCount(0)
  })

  test('read-only permissions survive a reload and are used to create the Agent', async ({ page }) => {
    await seedMock(page, { step_ms: 250 })
    await page.goto('/?scenario=normal')
    let wizard = await openGuideWizard(page)
    await wizard.getByTestId('agent-setup-name').fill('reader')
    await expect(wizard.getByTestId('agent-setup-name-available')).toBeVisible()
    await nextPage(wizard)
    await wizard.getByTestId('agent-setup-permission-read-only').getByRole('radio').click()
    await expect(wizard.getByTestId('agent-setup-permission-read-write').getByRole('radio')).not.toBeChecked()

    await page.reload()
    wizard = await openGuideWizard(page)
    await expect(wizard.getByTestId('agent-setup-permission-read-only').getByRole('radio')).toBeChecked()
    await nextPage(wizard)
    await expect(wizard.getByTestId('agent-setup-runtime-summary')).toContainText('Read only')
    await nextPage(wizard)
    await wizard.getByTestId('agent-setup-skip-channel').click()
    const permission = wizard.getByTestId('agent-setup-confirm-permission')
    await expect(permission).toContainText('Read only')
    await expect(permission).toContainText('always has read and write access to its own working directory')
    await wizard.getByTestId('agent-setup-create').click()
    await expect(wizard.getByTestId('agent-setup-status-success')).toBeVisible({ timeout: 15_000 })
    expect(await page.evaluate((key) => {
      const state = JSON.parse(window.localStorage.getItem(key) ?? '{}')
      return state.agents.find((agent: { name: string }) => agent.name === 'reader')?.permission_mode
    }, MOCK_KEY)).toBe('read_only')
  })

  test('Telegram needs the Owner identity; it is added on the profile and re-checked without losing the draft', async ({ page }) => {
    await seedMock(page, { step_ms: 250 })
    await page.goto('/?scenario=normal')
    const wizard = await openGuideWizard(page)
    await wizard.getByTestId('agent-setup-name').fill('tgbot')
    await expect(wizard.getByTestId('agent-setup-name-available')).toBeVisible()
    await nextPage(wizard)
    await nextPage(wizard)
    await expect(wizard.getByRole('combobox', { name: 'Agent template' })).toBeVisible()
    await nextPage(wizard)

    await expect(wizard.getByTestId('agent-setup-telegram-identity')).toHaveText('Owner identity missing')
    await wizard.getByRole('button', { name: 'Open my profile' }).click()
    const users = page.getByTestId('window-users-agents')
    const social = users.getByTestId('social-accounts')
    await social.getByRole('button', { name: 'Add' }).click()
    await page.getByRole('dialog').getByRole('button', { name: /^Telegram/ }).click()
    await page.getByTestId('social-telegram-id').fill('@me')
    await expect(page.getByText('A Telegram user ID contains digits only.')).toBeVisible()
    await page.getByTestId('social-telegram-id').fill('424242424')
    await page.getByRole('dialog').getByRole('button', { name: 'Save' }).click()
    await expect(social.locator('[data-platform="telegram"]')).toContainText('424242424')

    await page.getByTestId('window-drag-agent-setup').click()
    await expect(wizard.getByTestId('agent-setup-telegram-identity')).toHaveText('Owner identity configured')
    await wizard.getByTestId('agent-setup-channel-telegram').getByRole('radio').click()
    await expect(wizard.getByTestId('agent-setup-next')).toBeDisabled()
    await wizard.getByTestId('agent-setup-bot-token').fill('not-a-token')
    await expect(wizard.getByText('This does not look like a Telegram bot token.')).toBeVisible()
    await wizard.getByTestId('agent-setup-bot-token').fill(BOT_TOKEN)
    await expect(wizard.getByTestId('agent-setup-channel-status')).toContainText('Filled in')
    await nextPage(wizard)

    const channel = wizard.getByTestId('agent-setup-confirm-channel')
    await expect(channel).toContainText('Telegram')
    await expect(channel).toContainText('Bot Token filled in (hidden)')
    await expect(wizard).not.toContainText(BOT_TOKEN)
    await wizard.getByTestId('agent-setup-create').click()
    await expect(wizard.getByTestId('agent-setup-status-success')).toBeVisible({ timeout: 15_000 })
    await expect(wizard.getByTestId('agent-setup-status-channel')).toContainText('Channel bound')
  })

  test('a failed channel binding can be finished without the channel', async ({ page }) => {
    await seedMock(page, { step_ms: 250, user_bindings: [{ platform: 'telegram', account_id: '4242424242' }] })
    await page.goto('/?scenario=normal')
    const wizard = await openGuideWizard(page)
    await wizard.getByTestId('agent-setup-name').fill('tgfail')
    await expect(wizard.getByTestId('agent-setup-name-available')).toBeVisible()
    await nextPage(wizard)
    await nextPage(wizard)
    await nextPage(wizard)
    await wizard.getByTestId('agent-setup-channel-telegram').getByRole('radio').click()
    await wizard.getByTestId('agent-setup-bot-token').fill('123456789:FAILdqTcvCH1vGWJxfSeofSAs0K5PALDsaw')
    await nextPage(wizard)
    await wizard.getByTestId('agent-setup-create').click()
    const failure = wizard.getByTestId('agent-setup-status-failure')
    await expect(failure).toContainText('binding the Telegram channel failed', { timeout: 15_000 })
    await expect(wizard.locator('[data-step="tunnel"]')).toHaveAttribute('data-phase', 'failed')
    await expect(wizard.getByTestId('agent-setup-tunnel-reason')).toContainText('Bot Token is invalid')
    await wizard.getByRole('button', { name: 'Finish without the channel' }).click()
    await expect(wizard.getByTestId('agent-setup-status-success')).toBeVisible({ timeout: 15_000 })
    await expect(wizard.locator('[data-step="tunnel"]')).toHaveAttribute('data-phase', 'skipped')
  })

  test('a failed creation can be cancelled and the draft comes back', async ({ page }) => {
    await seedMock(page, { step_ms: 250 })
    await page.goto('/?scenario=normal')
    const wizard = await openGuideWizard(page)
    await wizard.getByTestId('agent-setup-name').fill('fail-runtime')
    await expect(wizard.getByTestId('agent-setup-name-available')).toBeVisible()
    await wizard.getByTestId('agent-setup-nickname').fill('Broken')
    await nextPage(wizard)
    await nextPage(wizard)
    await nextPage(wizard)
    await wizard.getByTestId('agent-setup-skip-channel').click()
    await wizard.getByTestId('agent-setup-create').click()
    await expect(wizard.getByTestId('agent-setup-status-failure')).toBeVisible({ timeout: 15_000 })
    await expect(wizard.getByTestId('agent-setup-status-error')).toContainText('image pull timed out')
    await expect(page.getByTestId('desktop-app-badge-agent-guide')).toBeVisible()
    await wizard.getByRole('button', { name: 'Cancel creation' }).click()
    await wizard.getByTestId('agent-setup-discard-confirm').getByRole('button', { name: 'Cancel creation' }).click()
    await expect(wizard.getByTestId('agent-setup-confirm-name')).toHaveText('Broken')
    await expect(page.getByTestId('desktop-app-badge-agent-guide')).toHaveCount(0)
  })

  test('the draft survives a reload and templates refresh after a CLI install', async ({ page }) => {
    await page.goto('/?scenario=normal')
    let wizard = await openGuideWizard(page)
    await wizard.getByTestId('agent-setup-name').fill('keeper')
    await wizard.getByTestId('agent-setup-nickname').fill('Keeper')
    await expect(wizard.getByTestId('agent-setup-name-available')).toBeVisible()
    await nextPage(wizard)
    await nextPage(wizard)
    await expect(wizard.getByRole('combobox', { name: 'Agent template' })).toHaveText('Jarvis · Built in')

    await page.reload()
    wizard = await openGuideWizard(page)
    await expect(wizard.getByRole('combobox', { name: 'Agent template' })).toBeVisible()
    await page.evaluate((key) => {
      const state = JSON.parse(window.localStorage.getItem(key) ?? '{}')
      state.templates = [...state.templates, {
        template_id: 'installed:coder.example', source: 'installed', app_id: 'coder.example', app_did: 'did:bns:coder.example',
        name: 'coder', show_name: 'Coder', description: 'Writes and reviews code.', version: '1.2.0', icon: null, loader: 'opendan', is_default: false,
      }]
      window.localStorage.setItem(key, JSON.stringify(state))
    }, MOCK_KEY)
    await wizard.getByRole('button', { name: 'Install other Agent templates' }).click()
    await expect(wizard.getByTestId('agent-setup-install-template')).toContainText('buckyos app fetch <pikg> --plan <plan.json>')
    await expect(wizard.getByTestId('agent-setup-install-template')).toContainText('--policy local-developer')
    await wizard.getByRole('button', { name: 'Refresh template list' }).click()
    await wizard.getByRole('combobox', { name: 'Agent template' }).click()
    await page.getByRole('option', { name: 'Coder · Installed by me' }).click()
    await expect(wizard.getByTestId('agent-setup-template-info')).toContainText('Writes and reviews code.')
    await wizard.getByRole('button', { name: 'Back' }).click()
    await wizard.getByRole('button', { name: 'Back' }).click()
    await expect(wizard.getByTestId('agent-setup-name')).toHaveValue('keeper')
    await expect(wizard.getByTestId('agent-setup-nickname')).toHaveValue('Keeper')
    await expect(wizard.getByTestId('agent-setup-bio')).toHaveValue('Writes and reviews code.')
  })

  test('from the guide an untaken suggestion replaces a taken “jarvis”', async ({ page }) => {
    await seedMock(page, { user_names: ['alice', 'bob', 'jarvis'] })
    await page.goto('/?scenario=normal')
    const wizard = await openGuideWizard(page)
    await expect(wizard.getByTestId('agent-setup-name')).toHaveValue('alice-jarvis')
    await expect(wizard.getByTestId('agent-setup-did')).toHaveText('did:bns:alice-jarvis.alice')
    await expect(wizard.getByTestId('agent-setup-nickname')).toHaveValue('Jarvis')
  })

  test('a name taken before submit sends the user back to the user name, keeping the rest', async ({ page }) => {
    await page.goto('/?scenario=normal')
    const wizard = await openGuideWizard(page)
    await wizard.getByTestId('agent-setup-name').fill('racer')
    await wizard.getByTestId('agent-setup-nickname').fill('Racer')
    await expect(wizard.getByTestId('agent-setup-name-available')).toBeVisible()
    await nextPage(wizard)
    await nextPage(wizard)
    await nextPage(wizard)
    await wizard.getByTestId('agent-setup-skip-channel').click()
    await page.evaluate((key) => {
      const state = JSON.parse(window.localStorage.getItem(key) ?? '{}')
      state.user_names = [...(state.user_names ?? ['alice']), 'racer']
      window.localStorage.setItem(key, JSON.stringify(state))
    }, MOCK_KEY)
    await wizard.getByTestId('agent-setup-create').click()
    await expect(wizard.getByTestId('agent-setup-name-conflict')).toBeVisible()
    await expect(wizard.getByTestId('agent-setup-name')).toHaveValue('racer')
    await expect(wizard.getByTestId('agent-setup-name-unavailable')).toContainText('alice-racer')
    await expect(wizard.getByTestId('agent-setup-nickname')).toHaveValue('Racer')
    await expect(wizard.getByTestId('agent-setup-next')).toBeDisabled()
  })

  test('Add Agent uses the same wizard; a taken name offers the Owner-prefixed suggestion', async ({ page }) => {
    await page.goto('/?scenario=normal')
    await page.getByTestId('desktop-app-users-agents').click()
    await page.getByTestId('window-users-agents').getByTestId('users-agents-add-agent').click()
    const wizard = page.getByTestId('window-agent-setup')
    await expect(wizard.getByTestId('agent-setup-name')).toHaveValue('')
    await expect(wizard.getByTestId('agent-setup-nickname')).toHaveValue('')
    await wizard.getByTestId('agent-setup-name').fill('assistant')
    await expect(wizard.getByTestId('agent-setup-name-unavailable')).toContainText('An Agent already has this name.')
    await wizard.getByRole('button', { name: 'alice-assistant' }).click()
    await expect(wizard.getByTestId('agent-setup-did')).toHaveText('did:bns:alice-assistant.alice')
    await wizard.getByTestId('agent-setup-name').fill('Bad_Name')
    await expect(wizard.getByTestId('agent-setup-name')).toHaveValue('bad_name')
    await expect(wizard.getByText('Use 1–63 lowercase letters, digits or hyphens')).toBeVisible()
    await expect(wizard.getByTestId('agent-setup-next')).toBeDisabled()
  })
})

test.describe('Agent details and group chats', () => {
  test.use({ viewport: { width: 1440, height: 900 } })

  test('details toggle group chats, MessageHub explains a disabled Agent, and delete asks in the page', async ({ page }) => {
    await page.goto('/?scenario=normal')
    await page.getByTestId('desktop-app-messagehub').click()
    const hub = page.getByTestId('window-messagehub')
    await hub.getByTestId('hub-create-menu').click()
    await hub.getByTestId('new-group').click()
    const form = hub.getByTestId('create-group-form')
    await form.getByRole('searchbox', { name: 'Search people' }).fill('Bucky')
    await form.getByRole('button', { name: 'BuckyOS Assistant', exact: true }).click()
    await form.getByRole('button', { name: 'Create group', exact: true }).click()
    await expect(form.getByRole('alert')).toContainText('not allowed to join group chats')
    await form.getByTestId('agent-group-settings-link').click()

    const users = page.getByTestId('window-users-agents')
    const detail = users.getByTestId('agent-detail')
    await expect(detail).toHaveAttribute('data-agent-id', 'assistant.alice')
    const group = detail.getByRole('switch', { name: 'Allow this Agent to join group chats' })
    await expect(group).not.toBeChecked()
    await group.click()
    await expect(group).toBeChecked()

    await page.getByTestId('window-drag-messagehub').click()
    await form.getByRole('button', { name: 'Create group', exact: true }).click()
    await expect(form).toHaveCount(0)

    await hub.getByRole('button', { name: 'Close' }).first().click()
    await detail.getByTestId('agent-delete').getByRole('button', { name: 'Delete Agent' }).click()
    await expect(detail.getByText('Delete “BuckyOS Assistant”? This cannot be undone.')).toBeVisible()
    await detail.getByTestId('agent-delete').getByRole('button', { name: 'Delete Agent' }).click()
    await expect(users.getByTestId('agent-detail')).toHaveCount(0)
    await expect(users.getByText('BuckyOS Assistant')).toHaveCount(0)
  })
})
