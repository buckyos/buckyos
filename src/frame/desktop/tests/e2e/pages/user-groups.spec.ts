import { expect, test, type Locator, type Page } from '@playwright/test'

const MOCK_CONTROL_PANEL = 'buckyos.mock.control-panel.v1'

function trackConsole(page: Page) {
  const errors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text())
  })
  page.on('pageerror', error => errors.push(String(error)))
  return errors
}

/** `window.confirm` / `window.prompt` answer yes, with `answer` for prompts. */
function acceptDialogs(page: Page, answer = '') {
  page.on('dialog', dialog => void dialog.accept(dialog.type() === 'prompt' ? answer : undefined))
}

async function openUsersAgents(page: Page, theme = 'light') {
  await page.addInitScript(value => window.localStorage.setItem('buckyos.prototype.theme.v1', value), theme)
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-users-agents').click()
  const win = page.getByTestId('window-users-agents')
  await expect(win).toBeVisible()
  return win
}

async function openGroups(win: Locator) {
  await win.getByTestId('users-agents-user-groups').click()
  const groups = win.getByTestId('user-groups-page')
  await expect(groups).toBeVisible()
  return groups
}

async function createGroup(groups: Locator, id: string, name: string) {
  const form = groups.getByTestId('user-group-create')
  await form.getByLabel('Group ID').fill(id)
  await form.getByLabel('Name', { exact: true }).fill(name)
  await form.getByRole('button', { name: 'Create group' }).click()
  await expect(groups.locator(`[data-group-id="${id}"]`)).toContainText(name)
}

test.describe('System user groups', () => {
  test('admin manages groups, members and projection rules', async ({ page }) => {
    const errors = trackConsole(page)
    acceptDialogs(page, 'Customers (VIP)')
    const win = await openUsersAgents(page)
    await expect(win.getByTestId('users-agents-user-groups')).toContainText('0 groups')
    const groups = await openGroups(win)

    const projections = groups.getByTestId('contact-projections')
    await expect(projections.getByTestId('projection-mode')).toHaveText('Default rules')
    await expect(projections.getByTestId('projection-rule')).toHaveCount(4)
    await expect(projections.getByTestId('projection-rule').first()).toContainText('All admins appear in the address book of All admins')

    const form = groups.getByTestId('user-group-create')
    await form.getByLabel('Group ID').fill('Bad id')
    await expect(form).toContainText('Start with a lowercase letter')
    await createGroup(groups, 'customer', 'Customers')
    await createGroup(groups, 'service', 'Service desk')
    await expect(win.getByTestId('users-agents-user-groups')).toContainText('2 groups')

    const customer = groups.locator('[data-group-id="customer"]')
    await customer.getByLabel('Add a member to Customers').selectOption('dave')
    await expect(customer).toContainText('Members (1)')
    await expect(customer).toContainText('Dave.Alice')
    const service = groups.locator('[data-group-id="service"]')
    await service.getByLabel('Add a member to Service desk').selectOption('bob')
    await expect(service).toContainText('Bob')

    await projections.getByLabel('Source group').selectOption('group:customer')
    await projections.getByLabel('Shown to').selectOption('group:service')
    await projections.getByRole('button', { name: 'Add rule' }).click()
    await expect(projections.getByTestId('projection-mode')).toHaveText('Custom rules')
    await expect(projections.getByTestId('projection-rule')).toHaveCount(5)
    await expect(projections.getByTestId('projection-rule').last()).toContainText('Customers appear in the address book of Service desk')
    await projections.getByLabel('Source group').selectOption('group:customer')
    await projections.getByLabel('Shown to').selectOption('group:service')
    await expect(projections).toContainText('This rule exists already.')

    await projections.getByLabel('Remove rule All admins → All admins').click()
    await expect(projections.getByTestId('projection-rule')).toHaveCount(4)

    await page.screenshot({ path: 'test-results/user-groups-light.png' })

    await customer.getByLabel('Rename Customers').click()
    await expect(groups.locator('[data-group-id="customer"]')).toContainText('Customers (VIP)')
    await groups.locator('[data-group-id="customer"]').getByLabel('Remove Dave.Alice').click()
    await expect(groups.locator('[data-group-id="customer"]')).toContainText('No members.')

    await groups.locator('[data-group-id="customer"]').getByLabel('Delete Customers (VIP)').click()
    await expect(groups.locator('[data-group-id="customer"]')).toHaveCount(0)
    await expect(projections.getByTestId('projection-rule')).toHaveCount(3)

    await projections.getByRole('button', { name: 'Restore default rules' }).click()
    await expect(projections.getByTestId('projection-mode')).toHaveText('Default rules')
    await expect(projections.getByTestId('projection-rule')).toHaveCount(4)
    expect(errors).toEqual([])
  })

  test('user detail: account type, per-account permissions and group membership', async ({ page }) => {
    const errors = trackConsole(page)
    acceptDialogs(page)
    const win = await openUsersAgents(page)
    const groups = await openGroups(win)
    await createGroup(groups, 'customer', 'Customers')

    await win.locator('[data-entity-id="dave"]').click()
    const account = win.getByTestId('user-account-admin')
    await expect(account.getByLabel('Account type')).toHaveValue('limited')
    await expect(account).toContainText('has no app or data access of its own')
    const contacts = account.getByTestId('user-option-contacts')
    await expect(contacts.getByRole('switch')).not.toBeChecked()
    await expect(contacts).toContainText('Account type default')
    await contacts.getByRole('switch').click()
    await expect(contacts.getByRole('switch')).toBeChecked()
    await expect(contacts).toContainText('Set for this account')
    await contacts.getByRole('button', { name: 'Use the default' }).click()
    await expect(contacts).toContainText('Account type default')
    await expect(contacts.getByRole('switch')).not.toBeChecked()

    await account.getByLabel('Add to group…').selectOption('customer')
    await expect(account.getByTestId('user-groups')).toContainText('Customers')
    await account.getByLabel('Remove from Customers').click()
    await expect(account.getByTestId('user-groups')).toContainText('Not in any system user group.')

    const typeOptions = await account.getByLabel('Account type').locator('option').allTextContents()
    expect(typeOptions).toEqual(['Limited', 'Admin', 'User'])
    await account.getByLabel('Account type').selectOption('user')
    await expect(account.getByLabel('Account type')).toHaveValue('user')
    await expect(account.getByTestId('user-option-password').getByRole('switch')).toBeChecked()
    await account.screenshot({ path: 'test-results/user-account-admin.png' })
    expect(errors).toEqual([])
  })

  test('new user wizard offers Limited accounts and system groups', async ({ page }) => {
    const errors = trackConsole(page)
    const win = await openUsersAgents(page)
    const groups = await openGroups(win)
    await createGroup(groups, 'customer', 'Customers')

    await win.getByLabel('Add User').click()
    await win.getByLabel('Local username').fill('client1')
    await win.getByLabel('Display name').fill('Client One')
    await win.getByLabel('Initial password').fill('local-password-1')
    await win.getByLabel('Confirm password').fill('local-password-1')
    await win.getByLabel('User type').selectOption('limited')
    await expect(win.getByText('No access of its own until you add it')).toBeVisible()
    await win.getByTestId('new-user-groups').getByLabel('Customers').check()
    await win.getByRole('button', { name: 'Next' }).click()
    const review = win.locator('form')
    await expect(review).toContainText('Limited')
    await expect(review).toContainText('Customers')
    await expect(review).toContainText('Not allowed')
    expect(errors).toEqual([])
  })

  test('dark theme keeps the group page legible', async ({ page }) => {
    const errors = trackConsole(page)
    const win = await openUsersAgents(page, 'dark')
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')
    const groups = await openGroups(win)
    await createGroup(groups, 'service', 'Service desk')
    await groups.locator('[data-group-id="service"]').getByLabel('Add a member to Service desk').selectOption('bob')
    await expect(groups.locator('[data-group-id="service"]')).toContainText('Members (1)')
    await page.screenshot({ path: 'test-results/user-groups-dark.png' })
    expect(errors).toEqual([])
  })

  test('accounts that are not admins see no group management', async ({ page }) => {
    await page.addInitScript(([key]) => {
      window.localStorage.setItem(key, JSON.stringify({
        account: { user_id: 'bob', user_name: 'Bob', user_type: 'user', did: 'did:bns:bob' },
      }))
    }, [MOCK_CONTROL_PANEL])
    const win = await openUsersAgents(page)
    await expect(win.getByText('Internal Entities')).toBeVisible()
    await expect(win.getByTestId('users-agents-user-groups')).toHaveCount(0)
  })
})

test.describe('My Network projection entries', () => {
  async function openMyNetwork(page: Page) {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/?scenario=normal')
    await page.getByTestId('desktop-app-my-network').click()
    const win = page.getByTestId('window-my-network')
    await expect(win.getByTestId('my-network-home')).toBeVisible()
    return win
  }

  test('a projected contact cannot be deleted but can still be tagged or blocked', async ({ page }) => {
    const errors = trackConsole(page)
    const win = await openMyNetwork(page)
    await expect(win.getByTestId('nav-collection-type:user')).toContainText('Zone members')
    await win.getByTestId('nav-contacts').click()
    const bob = win.getByTestId('my-network-person').filter({ has: page.getByRole('button', { name: 'Bob', exact: true }) })
    await expect(bob.getByTestId('person-projected')).toHaveText('Zone')
    await expect(bob.getByRole('checkbox')).toBeEnabled()
    await win.getByRole('button', { name: 'Bob', exact: true }).click()
    const detail = win.getByTestId('contact-detail')
    await expect(detail.getByTestId('contact-projected')).toContainText('Zone members')
    await expect(detail.getByTestId('relation-actions').getByRole('button', { name: 'Delete contact' })).toHaveCount(0)
    await expect(detail.getByTestId('contact-collections')).toContainText('Zone members')
    await expect(detail.getByTestId('relation-actions').getByRole('button', { name: 'Block' })).toBeVisible()
    expect(errors).toEqual([])
  })

  test('an account that may not edit its address book sees only the projection', async ({ page }) => {
    const errors = trackConsole(page)
    await page.addInitScript(([key]) => {
      window.localStorage.setItem(key, JSON.stringify({
        account: { user_id: 'dave', user_name: 'Dave', user_type: 'limited', did: 'did:bns:dave.alice' },
      }))
    }, [MOCK_CONTROL_PANEL])
    const win = await openMyNetwork(page)
    await expect(win.getByTestId('my-network-add-friend')).toHaveCount(0)
    await expect(win.getByTestId('my-network-create-collection')).toHaveCount(0)
    await expect(win.getByTestId('my-network-nav-read-only')).toBeVisible()
    await win.getByTestId('nav-contacts').click()
    await expect(win.getByTestId('my-network-read-only')).toBeVisible()
    await expect(win.getByTestId('my-network-person')).toHaveCount(1)
    await expect(win.getByRole('checkbox')).toHaveCount(0)
    await win.getByRole('button', { name: 'Bob', exact: true }).click()
    await expect(win.getByTestId('contact-read-only')).toBeVisible()
    await expect(win.getByTestId('relation-actions')).toHaveCount(0)
    await page.screenshot({ path: 'test-results/my-network-projection-read-only.png' })
    expect(errors).toEqual([])
  })
})
