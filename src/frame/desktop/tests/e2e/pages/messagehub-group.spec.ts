import { expect, test, type Page } from '@playwright/test'

declare global {
  interface Window { __messageHubMock: import('../../../src/app/messagehub/mock/store').MessageHubMockStore }
}

const SELF = 'did:buckyos:user:self'
const OWN = { viewerDid: SELF, ownerDid: SELF, mode: 'self' as const }
const BOB = 'did:buckyos:person:bob'
const HIKING = 'did:buckyos:group:weekend-hiking'
const TEAM = 'did:buckyos:group:product-team'

async function openHub(page: Page, entityId?: string) {
  await page.route('https://upload.wikimedia.org/**', route => route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"/>' }))
  await page.goto(entityId ? `/messagehub?entityId=${encodeURIComponent(entityId)}` : '/messagehub')
  await expect(page.getByRole('button', { name: 'New group', exact: true }).first()).toBeVisible()
}

test('a group is created from picked contacts and opens on its own conversation', async ({ page }) => {
  await openHub(page)
  await page.getByTestId('new-group').click()
  const dialog = page.getByRole('dialog', { name: 'New group' })
  const picker = dialog.getByTestId('group-member-picker')
  await expect(picker.getByText('Carol Wang')).toHaveCount(0)
  await dialog.getByRole('searchbox', { name: 'Search contacts' }).fill('ali')
  await picker.getByRole('checkbox').first().check()
  await dialog.getByRole('searchbox', { name: 'Search contacts' }).fill('')
  await picker.locator('label', { hasText: 'Bob Zhang' }).getByRole('checkbox').check()
  await expect(dialog.getByTestId('group-selected-members').getByRole('button')).toHaveCount(2)
  await expect(dialog.getByRole('textbox', { name: 'Group name' })).toHaveAttribute('placeholder', 'Alice Chen, Bob Zhang')
  await page.screenshot({ path: 'test-results/messagehub-group-create.png' })
  await dialog.getByRole('textbox', { name: 'Group name' }).fill('Launch crew')
  await dialog.getByRole('button', { name: 'Create group', exact: true }).click()
  await expect(dialog).toHaveCount(0)

  const groupDid = await page.evaluate(context => window.__messageHubMock.entities(context).find(entity => entity.name === 'Launch crew')?.id, OWN)
  expect(groupDid).toMatch(/^did:buckyos:group:/)
  await expect(page.getByRole('button', { name: 'Entity details: Launch crew' }).last()).toBeVisible()
  await expect(page.getByTestId('action-message')).toHaveText(['You created the group', 'You invited Alice Chen', 'You invited Bob Zhang'])

  await page.locator('textarea').fill('Hello crew')
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('conversation-history').getByText('Hello crew')).toBeVisible()
  const sent = await page.evaluate(({ context, id }) => window.__messageHubMock.reader(context, id).readRange(0, 10), { context: OWN, id: groupDid! })
  expect(sent.at(-1)).toMatchObject({ kind: 'group_msg', to: [groupDid], content: { content: 'Hello crew' } })

  await page.getByRole('button', { name: 'Entity details: Launch crew' }).last().click()
  const panel = page.getByTestId('group-panel')
  await expect(panel.getByTestId('group-members').locator('li')).toHaveCount(3)
  await expect(panel.locator('li', { hasText: 'You' })).toContainText('Owner')
  await expect(panel.locator('li', { hasText: 'Alice Chen' })).toContainText('Invited')
  await page.screenshot({ path: 'test-results/messagehub-group-panel.png' })

  await page.evaluate(({ group, alice }) => window.__messageHubMock.simulateJoin(group, alice), { group: groupDid!, alice: 'did:buckyos:person:alice' })
  await expect(panel.locator('li', { hasText: 'Alice Chen' })).not.toContainText('Invited')
  await expect(page.getByTestId('action-message').last()).toHaveText('Alice Chen joined')

  await panel.getByRole('button', { name: 'Invite' }).click()
  const invite = page.getByRole('dialog', { name: 'Invite to Launch crew' })
  await expect(invite.locator('label', { hasText: 'Alice Chen' })).toContainText('Member')
  await expect(invite.locator('label', { hasText: 'Alice Chen' }).getByRole('checkbox')).toBeDisabled()
  await invite.locator('label', { hasText: 'Dave Li' }).getByRole('checkbox').check()
  await invite.getByRole('button', { name: 'Invite', exact: true }).click()
  await expect(invite).toHaveCount(0)
  await expect(panel.locator('li', { hasText: 'Dave Li' })).toContainText('Invited')

  await panel.getByRole('button', { name: 'Cancel invitation: Dave Li' }).click()
  await page.getByRole('dialog', { name: 'Cancel invitation' }).getByRole('button', { name: 'Cancel invitation' }).click()
  await expect(panel.locator('li', { hasText: 'Dave Li' })).toHaveCount(0)

  await panel.getByRole('button', { name: 'Delete group' }).click()
  await page.getByRole('dialog', { name: 'Delete group' }).getByRole('button', { name: 'Delete group' }).click()
  await expect(panel).toContainText('This group has been deleted.')
  await expect(page.getByTestId('composer-readonly')).toContainText('You are not a member of this group.')
})

test('quick entry points: groups filter row, empty-state button and contact details preselect', async ({ page }) => {
  await openHub(page)
  await page.getByTestId('entity-filters').getByRole('button', { name: 'Groups' }).click()
  await page.getByTestId('entity-list-create-group').click()
  await expect(page.getByRole('dialog', { name: 'New group' })).toBeVisible()
  await page.getByRole('dialog', { name: 'New group' }).getByRole('button', { name: 'Cancel' }).click()

  await page.getByTestId('entity-filters').getByRole('button', { name: 'All' }).click()
  await page.getByRole('button', { name: /^Bob Zhang/ }).click()
  await page.getByRole('button', { name: 'Entity details: Bob Zhang' }).last().click()
  await page.getByTestId('entity-create-group').click()
  const dialog = page.getByRole('dialog', { name: 'New group' })
  await expect(dialog.getByTestId('group-selected-members')).toHaveText('Bob Zhang')
  await expect(dialog.getByRole('button', { name: 'Create group', exact: true })).toBeEnabled()
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  await expect(dialog).toHaveCount(0)
})

test('an invitation card joins the group and opens it; an admin manages the seeded group', async ({ page }) => {
  await openHub(page, BOB)
  const card = page.getByTestId('group-notice')
  await expect(card).toHaveAttribute('data-state', 'pending')
  await expect(card).toContainText('Bob Zhang invited you to join')
  await expect(card).toContainText('Weekend Hiking')
  expect(await page.evaluate(context => window.__messageHubMock.entities(context).some(entity => entity.id === 'did:buckyos:group:weekend-hiking'), OWN)).toBe(false)
  await page.screenshot({ path: 'test-results/messagehub-group-invitation.png' })
  await card.getByRole('button', { name: 'Join group' }).click()
  await expect(card).toHaveAttribute('data-state', 'joined')
  await card.getByRole('button', { name: 'Open' }).click()
  await expect(page.getByRole('button', { name: 'Entity details: Weekend Hiking' }).last()).toBeVisible()
  await expect(page.getByTestId('action-message').last()).toHaveText('You joined')
  await page.getByRole('button', { name: 'Entity details: Weekend Hiking' }).last().click()
  const panel = page.getByTestId('group-panel')
  await expect(panel.locator('li', { hasText: 'Bob Zhang' })).toContainText('Owner')
  await expect(panel.getByRole('button', { name: 'Invite' })).toHaveCount(0)
  await panel.getByRole('button', { name: 'Leave group' }).click()
  await page.getByRole('dialog', { name: 'Leave group' }).getByRole('button', { name: 'Leave group' }).click()
  await expect(page.getByTestId('composer-readonly')).toContainText('You are not a member of this group.')
  expect(await page.evaluate(({ context, group }) => window.__messageHubMock.group(context, group)?.myRole, { context: OWN, group: HIKING })).toBeUndefined()

  await page.getByRole('button', { name: /^Product Team/ }).click()
  await page.getByRole('button', { name: 'Entity details: Product Team' }).last().click()
  await expect(panel.locator('li', { hasText: 'Alice Chen' })).toContainText('Owner')
  await expect(panel.locator('li', { hasText: 'You' })).toContainText('Admin')
  await expect(panel.getByRole('button', { name: 'Remove from group: Alice Chen' })).toHaveCount(0)
  await panel.getByRole('button', { name: 'Remove from group: Dave Li' }).click()
  await page.getByRole('dialog', { name: 'Remove from group' }).getByRole('button', { name: 'Remove from group' }).click()
  await expect(panel.locator('li', { hasText: 'Dave Li' })).toHaveCount(0)
  expect(await page.evaluate(({ context, group }) => window.__messageHubMock.group(context, group)?.members?.find(member => member.did === 'did:buckyos:entity:person-dave')?.state, { context: OWN, group: TEAM })).toBe('removed')
})

test('group creation is a full-height sheet-friendly dialog on a phone', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 })
  await page.goto('/messagehub')
  await page.getByRole('button', { name: 'Back to conversation list', exact: true }).click()
  await page.getByTestId('new-group').click()
  const dialog = page.getByRole('dialog', { name: 'New group' })
  await expect(dialog.getByTestId('group-member-picker')).toBeVisible()
  const box = await dialog.boundingBox()
  expect(box!.x).toBeGreaterThanOrEqual(0)
  expect(box!.x + box!.width).toBeLessThanOrEqual(375)
  await page.screenshot({ path: 'test-results/messagehub-group-create-375.png' })
})
