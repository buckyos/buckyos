import { expect, test, type Locator, type Page } from '@playwright/test'

function trackConsole(page: Page) {
  const errors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text())
  })
  page.on('pageerror', error => errors.push(String(error)))
  return errors
}

async function openMyNetwork(page: Page, query = '') {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto(`/?scenario=normal${query}`)
  await page.getByTestId('desktop-app-my-network').click()
  const win = page.getByTestId('window-my-network')
  await expect(win.getByTestId('my-network-home')).toBeVisible()
  return win
}

async function openAddFriend(win: Locator) {
  await win.getByTestId('my-network-add-friend').click()
  const dialog = win.getByRole('dialog', { name: 'Add Friend / Contact' })
  await expect(dialog).toBeVisible()
  return dialog
}

async function findAndView(dialog: Locator, input: string, name: string) {
  await dialog.getByLabel('Find people').fill(input)
  const row = dialog.getByTestId('find-people-result').filter({ hasText: name }).first()
  await row.getByRole('button', { name: 'View', exact: true }).click()
  await expect(dialog.getByTestId('network-profile-card')).toContainText(name)
}

async function openJoinGroup(win: Locator) {
  await win.getByTestId('my-network-join-group').click()
  const dialog = win.getByRole('dialog', { name: 'Join Group' })
  await expect(dialog).toBeVisible()
  return dialog
}

async function previewGroup(dialog: Locator, input: string) {
  await dialog.getByLabel('Group DID or invite link').fill(input)
  await dialog.getByRole('button', { name: 'Find', exact: true }).click()
  await expect(dialog.getByTestId('join-group-preview')).toBeVisible()
}

function person(win: Locator, name: string) {
  return win.getByTestId('my-network-person').filter({ has: win.page().getByRole('button', { name, exact: true }) })
}

test.describe('My Network', () => {
  test('overview, navigation and collection sections', async ({ page }) => {
    const errors = trackConsole(page)
    const win = await openMyNetwork(page)
    await expect(win.getByTestId('metric-contacts')).toContainText('12')
    await expect(win.getByTestId('metric-friends')).toContainText('5')
    await expect(win.getByTestId('metric-groups')).toContainText('3')
    await expect(win.getByTestId('metric-requests')).toContainText('6')
    await expect(win.getByTestId('nav-requests')).toContainText('6')
    await expect(win.getByTestId('nav-collection-Project Alpha Team')).toBeVisible()
    await expect(win.getByTestId('nav-collection-homestation.following')).toContainText('People I follow')
    await expect(win.getByTestId('nav-collection-zone_user')).toContainText('Users of this Zone')

    await win.getByTestId('nav-contacts').click()
    await expect(win.getByTestId('my-network-person')).toHaveCount(12)
    await win.getByRole('button', { name: 'Eve', exact: true }).click()
    const detail = win.getByTestId('contact-detail')
    await expect(detail.getByTestId('contact-header')).toContainText('did:bns:eve')
    await expect(detail.getByTestId('relation-friend')).toBeVisible()
    await expect(detail.getByTestId('relation-following')).toBeVisible()
    await expect(detail.getByTestId('contact-send-message')).toBeVisible()
    await expect(detail.getByTestId('contact-home-page')).toHaveAttribute('href', 'https://eve.buckyos.io/homestation/eve')

    await win.getByTestId('nav-friends').click()
    await expect(win.getByTestId('my-network-person')).toHaveCount(5)
    await expect(win.getByRole('button', { name: 'Kevin', exact: true })).toHaveCount(0)

    await win.getByRole('button', { name: 'Eve', exact: true }).click()
    await win.getByTestId('contact-send-message').click()
    await expect(page.getByTestId('window-messagehub')).toBeVisible()
    await win.click({ position: { x: 600, y: 12 } })
    await win.getByTestId('nav-collection-homestation.following').click()
    await win.getByTestId('collection-manage-homestation').click()
    await expect(page.getByTestId('window-homestation')).toBeVisible()
    expect(errors).toEqual([])
  })

  test('add friend: same Zone, other Zone, queued and unreachable', async ({ page }) => {
    const errors = trackConsole(page)
    const win = await openMyNetwork(page)

    let dialog = await openAddFriend(win)
    await findAndView(dialog, 'bob', 'Bob')
    await expect(dialog.getByTestId('add-friend-already')).toContainText('same Zone')
    await expect(dialog.getByTestId('add-friend-send')).toHaveCount(0)
    await dialog.getByRole('button', { name: 'Back', exact: true }).click()

    await findAndView(dialog, 'dave', 'Dave')
    await dialog.getByTestId('add-friend-send').click()
    await expect(dialog.getByTestId('add-friend-outcome')).toHaveAttribute('data-outcome', 'friends')
    await expect(dialog.getByTestId('add-friend-outcome-text')).toContainText('Dave is in your Zone')
    await dialog.getByRole('button', { name: 'Done', exact: true }).click()
    await expect(win.getByTestId('contact-detail')).toHaveAttribute('data-did', 'did:bns:dave')
    await expect(win.getByTestId('contact-detail').getByTestId('relation-friend')).toBeVisible()

    dialog = await openAddFriend(win)
    await findAndView(dialog, 'did:bns:nora', 'Nora')
    await dialog.getByLabel('Message (optional)').fill('Hi Nora, Alice from the meetup.')
    await dialog.getByLabel('Collection name').fill('Meetup')
    await dialog.getByRole('button', { name: 'Add', exact: true }).click()
    await dialog.getByTestId('add-friend-send').click()
    await expect(dialog.getByTestId('add-friend-outcome')).toHaveAttribute('data-outcome', 'request_pending')
    await expect(dialog.getByTestId('add-friend-outcome-text')).toContainText('Waiting for Nora to confirm')
    await dialog.getByRole('button', { name: 'Done', exact: true }).click()
    await expect(win.getByTestId('contact-detail').getByTestId('relation-outgoing_pending')).toBeVisible()
    await expect(win.getByTestId('nav-collection-Meetup')).toContainText('1')

    dialog = await openAddFriend(win)
    await findAndView(dialog, 'did:bns:oscar', 'Oscar')
    await dialog.getByTestId('add-friend-send').click()
    await expect(dialog.getByTestId('add-friend-outcome')).toHaveAttribute('data-outcome', 'request_queued')
    await expect(dialog.getByTestId('add-friend-outcome-text')).toContainText('Waiting for delivery')
    await dialog.getByRole('button', { name: 'Done', exact: true }).click()

    dialog = await openAddFriend(win)
    await findAndView(dialog, 'did:bns:quinn', 'Quinn')
    await expect(dialog.getByTestId('add-friend-unreachable')).toContainText('their Zone gateway does not answer')
    await expect(dialog.getByTestId('add-friend-send')).toBeDisabled()
    await expect(dialog.getByTestId('copy-share-link')).toBeVisible()
    await dialog.getByRole('button', { name: 'Back', exact: true }).click()
    await dialog.getByLabel('Find people').fill('did:bns:nobody')
    await expect(dialog.getByTestId('find-people-lookup-error')).toContainText('Nobody with this DID or name was found')
    await dialog.getByLabel('Find people').fill('%%%')
    await expect(dialog.getByTestId('find-people-lookup-error')).toContainText('not a DID, name or link')
    await dialog.getByRole('button', { name: 'Close dialog' }).click()

    await win.getByTestId('nav-requests').click()
    const sent = win.getByTestId('requests-friendOutgoing')
    await expect(sent.getByTestId('request-friend-outgoing')).toHaveCount(3)
    await expect(sent).toContainText('Nora')
    await expect(sent.getByTestId('request-friend-outgoing').filter({ hasText: 'Oscar' })).toContainText('Waiting for delivery')
    const ivan = sent.getByTestId('request-friend-outgoing').filter({ hasText: 'Ivan' })
    await expect(ivan).toContainText('Not delivered')
    await ivan.getByRole('button', { name: 'Send again', exact: true }).click()
    await expect(ivan).toContainText('Waiting for delivery')
    await expect(ivan.getByRole('button', { name: 'Send again', exact: true })).toHaveCount(0)
    expect(errors).toEqual([])
  })

  test('requests: friend requests and message requests', async ({ page }) => {
    const errors = trackConsole(page)
    const win = await openMyNetwork(page)
    await win.getByTestId('nav-requests').click()
    const incoming = win.getByTestId('request-friend-incoming')
    await expect(incoming).toHaveCount(2)

    await incoming.filter({ hasText: 'Nina' }).getByRole('button', { name: 'Accept', exact: true }).click()
    await expect(incoming).toHaveCount(1)
    await incoming.filter({ hasText: 'Peggy' }).getByRole('button', { name: 'Ignore', exact: true }).click()
    await expect(win.getByTestId('requests-friendIncoming')).toHaveCount(0)

    const messages = win.getByTestId('request-message')
    await expect(messages).toHaveCount(2)
    await expect(messages.filter({ hasText: 'Victor' })).toContainText('sent 2 message(s)')
    await messages.filter({ hasText: 'Victor' }).getByRole('button', { name: 'Allow messages', exact: true }).click()
    await expect(messages).toHaveCount(1)
    await messages.filter({ hasText: 'Wendy' }).getByRole('button', { name: 'Block', exact: true }).click()
    const confirm = win.getByRole('dialog', { name: 'Block Wendy?' })
    await expect(confirm.getByTestId('network-confirm')).toHaveAttribute('data-strength', 'explicit')
    await confirm.getByTestId('network-confirm-submit').click()
    await expect(win.getByTestId('requests-message')).toHaveCount(0)

    await expect(win.getByTestId('request-group-invitation')).toContainText('Hiking Club')
    await win.getByTestId('request-group-invitation').getByRole('button', { name: 'Join', exact: true }).click()
    await expect(win.getByTestId('request-group-invitation')).toHaveCount(0)
    await win.getByTestId('request-join').getByRole('button', { name: 'Approve', exact: true }).click()
    await expect(win.getByTestId('request-join')).toHaveCount(0)
    await expect(win.getByTestId('nav-requests')).not.toContainText(/[1-9]/)

    await win.getByTestId('nav-friends').click()
    await expect(win.getByRole('button', { name: 'Nina', exact: true })).toBeVisible()
    await win.getByTestId('nav-contacts').click()
    await expect(person(win, 'Wendy').getByTestId('relation-blocked')).toBeVisible()
    await expect(person(win, 'Victor').getByTestId('relation-contact')).toBeVisible()
    await win.getByTestId('nav-groups').click()
    await expect(win.getByRole('button', { name: 'Hiking Club', exact: true })).toBeVisible()
    expect(errors).toEqual([])
  })

  test('join group: joined, pending approval, invite required and other Zone', async ({ page }) => {
    const errors = trackConsole(page)
    const win = await openMyNetwork(page)

    let dialog = await openJoinGroup(win)
    await previewGroup(dialog, 'did:bns:design-guild')
    await expect(dialog.getByTestId('join-group-preview')).toContainText('Admins approve new members')
    await dialog.getByTestId('join-group-submit').click()
    await expect(dialog.getByTestId('join-group-outcome')).toHaveAttribute('data-outcome', 'pending_admin_approval')
    await expect(dialog.getByTestId('join-group-outcome-text')).toContainText('has to approve it')
    await dialog.getByRole('button', { name: 'Done', exact: true }).click()

    dialog = await openJoinGroup(win)
    await previewGroup(dialog, 'did:bns:core-team')
    await dialog.getByTestId('join-group-submit').click()
    await expect(dialog.getByTestId('join-group-error')).toContainText('only accepts invited members')
    await previewGroup(dialog, 'did:bns:core-team?invite=core-2026')
    await expect(dialog.getByTestId('join-group-preview')).toContainText('Invite link')
    await dialog.getByTestId('join-group-submit').click()
    await expect(dialog.getByTestId('join-group-outcome')).toHaveAttribute('data-outcome', 'active')
    await dialog.getByRole('button', { name: 'Done', exact: true }).click()
    await expect(win.getByTestId('group-detail')).toHaveAttribute('data-did', 'did:bns:core-team')

    dialog = await openJoinGroup(win)
    await previewGroup(dialog, 'did:bns:garden.eve')
    await expect(dialog.getByTestId('join-group-remote')).toContainText('not supported yet')
    await expect(dialog.getByTestId('join-group-submit')).toHaveCount(0)
    await dialog.getByRole('button', { name: 'Close', exact: true }).click()

    dialog = await openJoinGroup(win)
    await previewGroup(dialog, 'did:bns:book-club')
    await expect(dialog.getByTestId('join-group-preview')).toContainText('Anyone can join')
    await expect(dialog.getByTestId('join-group-preview')).toContainText('6 members')
    await dialog.getByTestId('join-group-submit').click()
    await expect(dialog.getByTestId('join-group-outcome-text')).toContainText('You joined Book Club')
    await dialog.getByRole('button', { name: 'Done', exact: true }).click()
    const detail = win.getByTestId('group-detail')
    await expect(detail).toHaveAttribute('data-did', 'did:bns:book-club')
    await expect(detail).toContainText('Hosted by this Zone')
    await expect(detail.getByTestId('group-open-messagehub')).toBeVisible()
    await expect(win.getByTestId('my-network-group')).toHaveCount(6)
    await expect(win.getByTestId('my-network-group').filter({ hasText: 'Design Guild' })).toContainText('Waiting for approval')
    await expect(win.getByTestId('nav-groups')).toContainText('5')

    await win.getByRole('button', { name: 'Project Alpha', exact: true }).click()
    await win.getByTestId('group-leave').click()
    await win.getByRole('dialog', { name: 'Leave Project Alpha?' }).getByTestId('network-confirm-submit').click()
    await expect(win.getByTestId('group-error')).toContainText('Transfer the group')
    expect(errors).toEqual([])
  })

  test('user collection add and remove; collections are tags', async ({ page }) => {
    const errors = trackConsole(page)
    const win = await openMyNetwork(page)
    await win.getByTestId('nav-collection-Project Alpha Team').click()
    await expect(win.getByTestId('my-network-list-title')).toHaveText('Project Alpha Team')
    await expect(win.getByTestId('my-network-person')).toHaveCount(3)

    await win.getByTestId('collection-add-members').click()
    const picker = win.getByRole('dialog', { name: 'Add people to “Project Alpha Team”' })
    await expect(picker.getByTestId('people-picker-row').filter({ hasText: 'Eve' })).toContainText('Already in this collection')
    await picker.getByRole('button', { name: 'Heidi', exact: true }).click()
    await picker.getByRole('button', { name: 'Mallory', exact: true }).click()
    await expect(picker.getByTestId('people-picker-selected')).toContainText('Heidi')
    await picker.getByRole('button', { name: 'Add to collection', exact: true }).click()
    await expect(win.getByTestId('my-network-person')).toHaveCount(5)

    await win.getByRole('button', { name: 'Lily', exact: true }).click()
    await win.getByTestId('relation-actions').getByRole('button', { name: 'Remove from collection' }).click()
    const confirm = win.getByRole('dialog', { name: 'Remove from collection?' })
    await expect(confirm.getByTestId('network-confirm')).toHaveAttribute('data-strength', 'light')
    await expect(confirm).toContainText('They stay in your Contacts')
    await confirm.getByTestId('network-confirm-submit').click()
    await expect(win.getByTestId('my-network-person')).toHaveCount(4)
    await win.getByTestId('nav-contacts').click()
    await expect(win.getByRole('button', { name: 'Lily', exact: true })).toBeVisible()

    await win.getByTestId('my-network-create-collection').click()
    const nameDialog = win.getByRole('dialog', { name: 'New collection' })
    await nameDialog.getByLabel('Collection name').fill('zone_user')
    await nameDialog.getByRole('button', { name: 'Choose members' }).click()
    await expect(nameDialog.getByRole('alert')).toContainText('reserved')
    await nameDialog.getByLabel('Collection name').fill('Family')
    await nameDialog.getByRole('button', { name: 'Choose members' }).click()
    const members = win.getByRole('dialog', { name: 'Add people to “Family”' })
    await members.getByRole('button', { name: 'Carol', exact: true }).click()
    await members.getByRole('button', { name: 'Create collection' }).click()
    await expect(win.getByTestId('my-network-list-title')).toHaveText('Family')
    await expect(win.getByTestId('my-network-person')).toHaveCount(1)
    expect(errors).toEqual([])
  })

  test('“我关注的人” is visible and read-only', async ({ page }) => {
    const errors = trackConsole(page)
    await page.addInitScript(() => localStorage.setItem('buckyos.prototype.locale.v1', 'zh-CN'))
    const win = await openMyNetwork(page)
    const following = win.getByTestId('nav-collection-homestation.following')
    await expect(following).toContainText('我关注的人')
    await following.click()
    await expect(win.getByTestId('my-network-list-title')).toHaveText('我关注的人')
    await expect(win.getByTestId('my-network-person')).toHaveCount(2)
    await expect(win.getByText('由应用维护，这里只读。')).toBeVisible()
    await expect(win.getByTestId('collection-manage-homestation')).toHaveText('在 HomeStation 中管理')
    await expect(win.getByTestId('collection-add-members')).toHaveCount(0)
    await expect(win.getByTestId('collection-rename')).toHaveCount(0)
    await expect(win.getByTestId('collection-delete')).toHaveCount(0)
    await win.getByRole('button', { name: 'Lily', exact: true }).click()
    await expect(win.getByTestId('contact-detail').getByTestId('relation-following')).toHaveText('我关注的')
    await expect(win.getByTestId('relation-actions').getByRole('button', { name: '移出分组' })).toHaveCount(0)
    await expect(win.getByTestId('contact-collections')).toContainText('我关注的人')
    expect(errors).toEqual([])
  })

  test('delete semantics: delete contact, unfriend, block, system relation', async ({ page }) => {
    const errors = trackConsole(page)
    const win = await openMyNetwork(page)
    await win.getByTestId('nav-contacts').click()

    await win.getByRole('button', { name: 'Heidi', exact: true }).click()
    await win.getByTestId('relation-actions').getByRole('button', { name: 'Delete contact' }).click()
    let confirm = win.getByRole('dialog', { name: 'Delete Heidi from contacts?' })
    await expect(confirm.getByTestId('network-confirm')).toHaveAttribute('data-strength', 'explicit')
    await expect(confirm).toContainText('Message history is kept')
    await confirm.getByTestId('network-confirm-submit').click()
    await expect(win.getByRole('button', { name: 'Heidi', exact: true })).toHaveCount(0)
    await expect(win.getByTestId('my-network-person')).toHaveCount(11)

    await win.getByRole('button', { name: 'Frank', exact: true }).click()
    await win.getByTestId('relation-actions').getByRole('button', { name: 'Unfriend' }).click()
    confirm = win.getByRole('dialog', { name: 'Unfriend Frank?' })
    await expect(confirm.getByTestId('network-confirm')).toHaveAttribute('data-strength', 'strong')
    await expect(confirm.getByTestId('network-confirm-submit')).toBeDisabled()
    await expect(confirm.getByLabel('Tell Frank')).toBeChecked()
    await confirm.getByLabel('I understand this ends the friendship.').check()
    await confirm.getByTestId('network-confirm-submit').click()
    await expect(person(win, 'Frank').getByTestId('relation-contact')).toBeVisible()
    await expect(win.getByTestId('contact-detail').getByTestId('relation-friend')).toHaveCount(0)

    await win.getByRole('button', { name: 'Judy.telegram', exact: true }).click()
    await win.getByTestId('relation-actions').getByRole('button', { name: 'Block' }).click()
    confirm = win.getByRole('dialog', { name: 'Block Judy.telegram?' })
    await expect(confirm.getByTestId('network-confirm')).toHaveAttribute('data-strength', 'explicit')
    await confirm.getByTestId('network-confirm-submit').click()
    await expect(person(win, 'Judy.telegram').getByTestId('relation-blocked')).toBeVisible()
    await expect(win.getByTestId('relation-actions').getByRole('button', { name: 'Unblock' })).toBeVisible()

    await win.getByRole('button', { name: 'Carol', exact: true }).click()
    await win.getByTestId('relation-actions').getByRole('button', { name: 'Delete contact' }).click()
    await win.getByRole('dialog', { name: 'Delete Carol from contacts?' }).getByTestId('network-confirm-submit').click()
    await expect(win.getByTestId('relation-actions-error')).toContainText('set up by the system')
    await expect(win.getByRole('button', { name: 'Carol', exact: true })).toBeVisible()

    // A projection entry offers no delete at all.
    await win.getByRole('button', { name: 'Bob', exact: true }).click()
    await expect(win.getByTestId('contact-projected')).toBeVisible()
    await expect(win.getByTestId('relation-actions').getByRole('button', { name: 'Delete contact' })).toHaveCount(0)
    expect(errors).toEqual([])
  })

  test('load failure retries; empty network shows empty states', async ({ page }) => {
    const errors = trackConsole(page)
    let win = await openMyNetwork(page, '&networkScenario=error')
    await win.getByTestId('nav-contacts').click()
    await expect(win.getByTestId('my-network-list-error')).toContainText('msg-center cannot be reached')
    await win.getByTestId('my-network-list-error').getByRole('button', { name: 'Retry' }).click()
    await expect(win.getByTestId('my-network-person')).toHaveCount(12)

    win = await openMyNetwork(page, '&networkScenario=empty')
    await expect(win.getByTestId('home-empty')).toBeVisible()
    await win.getByTestId('nav-requests').click()
    await expect(win.getByTestId('requests-empty')).toBeVisible()
    await win.getByTestId('nav-groups').click()
    await expect(win.getByText('You have not joined any group yet.')).toBeVisible()
    expect(errors).toEqual([])
  })
})

test.describe('My Network mobile', () => {
  test.use({ viewport: { width: 375, height: 812 }, hasTouch: true, isMobile: true })

  test('mobile: views stack and the add friend dialog is a sheet', async ({ page }) => {
    const errors = trackConsole(page)
    await page.goto('/?scenario=normal')
    const app = page.getByRole('button', { name: 'My Network' }).first()
    await app.scrollIntoViewIfNeeded()
    const box = await app.boundingBox()
    const x = (box?.x ?? 0) + (box?.width ?? 0) / 2
    const y = (box?.y ?? 0) + (box?.height ?? 0) / 2
    await app.dispatchEvent('pointerdown', { bubbles: true, clientX: x, clientY: y, pointerId: 7, pointerType: 'touch' })
    await page.locator('body').dispatchEvent('pointerup', { bubbles: true, clientX: x + 4, clientY: y + 4, pointerId: 7, pointerType: 'touch' })

    const shell = page.getByTestId('my-network')
    await expect(shell).toHaveAttribute('data-stage', 'nav')
    await page.getByTestId('nav-requests').click()
    await expect(page.getByTestId('request-friend-incoming')).toHaveCount(2)
    await page.getByRole('button', { name: 'Back', exact: true }).click()
    await page.getByTestId('nav-contacts').click()
    await page.getByRole('button', { name: 'Eve', exact: true }).click()
    await expect(page.getByTestId('contact-detail')).toBeVisible()
    await page.getByRole('button', { name: 'Back', exact: true }).click()
    await page.getByRole('button', { name: 'Back', exact: true }).click()
    await page.getByTestId('my-network-add-friend').click()
    await expect(page.getByRole('dialog', { name: 'Add Friend / Contact' })).toBeVisible()

    const overflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth)
    expect(overflow).toBeFalsy()
    expect(errors).toEqual([])
  })
})
