import { expect, test, type Page } from '@playwright/test'
import { pickLocal, pickZone, putZoneFile, selectDialog } from '../fixtures/selectDialog'

/** A landscape picture drawn in the page, as a file the picker can take. */
async function landscapePicture(page: Page, name = 'sunset.jpg') {
  const base64 = await page.evaluate(() => {
    const canvas = document.createElement('canvas')
    canvas.width = 1200
    canvas.height = 800
    const context = canvas.getContext('2d')!
    context.fillStyle = '#1d3b6e'
    context.fillRect(0, 0, 1200, 400)
    context.fillStyle = '#e8875a'
    context.fillRect(0, 400, 1200, 400)
    return canvas.toDataURL('image/jpeg', 0.9).split(',')[1]
  })
  return { name, mimeType: 'image/jpeg', buffer: Buffer.from(base64, 'base64') }
}

const wallpaperLayer = (page: Page) => page.getByTestId('desktop-background').locator('.desktop-wallpaper-image')

/** The account's appearance state as the mock control panel holds it. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const accountAppearance = (page: Page): Promise<any> =>
  page.evaluate(() => JSON.parse(localStorage.getItem('buckyos.mock.desktop-state.appearance') ?? '{}'))

async function openSettings(page: Page, item?: string) {
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-settings').click()
  const win = page.getByTestId('window-settings')
  await expect(win.getByTestId('settings-general-page')).toBeVisible()
  if (item) await win.getByRole('button', { name: item, exact: true }).click()
  return win
}

test.describe('settings', () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 860 })
  })

  test('general: software and snapshot from the Zone, no intro block, no device info', async ({ page }) => {
    const win = await openSettings(page)
    await expect(win.getByTestId('settings-software-info')).toContainText('0.7.0')
    await expect(win.getByTestId('settings-system-snapshot')).toContainText('2 of 3 online')
    await expect(win.getByText('System information, device details, and support tools.')).toHaveCount(0)
    await expect(win.getByText('Device Info', { exact: true })).toHaveCount(0)

    const download = page.waitForEvent('download')
    await win.getByTestId('settings-export-system-info').click()
    expect((await download).suggestedFilename()).toMatch(/^buckyos-system-info-.*\.json$/)
  })

  test('search finds inner settings and retains compact icon-only selection', async ({ page }) => {
    const win = await openSettings(page)
    const search = win.getByRole('searchbox', { name: 'Search settings' })
    await search.fill('  ICON SIZE  ')
    const result = win.getByRole('button', { name: 'Appearance Desktop icon size' })
    await expect(result).toBeVisible()
    await result.click()
    await expect(win.getByTestId('settings-icon-size')).toBeVisible()
    await expect(result).toHaveAttribute('aria-current', 'page')
    await expect(result).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)')
    await search.fill('no-such-setting-xyz')
    await expect(win.getByText('No matching settings')).toBeVisible()
    await search.fill('')
    await expect(win.getByRole('button', { name: 'General', exact: true })).toBeVisible()
  })

  test('navigation names the Zone page "Zone Config"', async ({ page }) => {
    const win = await openSettings(page)
    await expect(win.getByRole('button', { name: 'Zone Config', exact: true })).toBeVisible()
    await expect(win.getByRole('button', { name: 'Zone Name & Icon' })).toHaveCount(0)
    await win.getByRole('button', { name: 'Zone Config', exact: true }).click()
    await expect(win.getByTestId('settings-zone-page')).toBeVisible()
  })

  test('appearance: no session management; the icon size persists', async ({ page }) => {
    const win = await openSettings(page, 'Appearance')
    await expect(win.getByTestId('settings-appearance-page')).toBeVisible()
    await expect(win.getByText('Session Management')).toHaveCount(0)
    await expect(win.getByText('Current Session')).toHaveCount(0)

    await win.getByTestId('settings-icon-size').selectOption('large')
    await page.reload()
    await page.getByTestId('desktop-app-settings').click()
    await page.getByTestId('window-settings').getByRole('button', { name: 'Appearance', exact: true }).click()
    await expect(page.getByTestId('settings-icon-size')).toHaveValue('large')
  })

  test('appearance: a desktop picture with its fit and position is saved to the account', async ({ page }) => {
    const win = await openSettings(page, 'Appearance')
    const section = win.getByTestId('settings-wallpaper')
    await expect(section).toContainText('saved to your account')
    await expect(section).toContainText('Using the default background')
    await expect(page.getByTestId('desktop-background')).not.toHaveAttribute('data-wallpaper-mode')
    await expect(page.locator('main[data-wallpaper]')).toHaveCount(0)

    // From the Zone, the user's main file system: the picker shows pictures as a grid.
    await putZoneFile(page, '/home/Pictures/sunset.jpg', (await landscapePicture(page)).buffer, 'image/jpeg')
    await win.getByTestId('settings-wallpaper-choose').click()
    await pickZone(selectDialog(page), '/home/Pictures/sunset.jpg')
    await expect(selectDialog(page)).toHaveCount(0)
    await expect(section.getByTestId('settings-wallpaper-status')).toContainText('sunset.jpg · 1200 × 800')
    await expect(page.getByTestId('desktop-background')).toHaveAttribute('data-wallpaper-mode', 'fill')
    await expect(wallpaperLayer(page)).toHaveCSS('background-image', /^url\("blob:/)
    await expect(page.locator('main[data-wallpaper="image"]')).toHaveCount(1)
    await expect(section.getByTestId('settings-wallpaper-preview-phone').locator('.desktop-wallpaper-image')).toHaveCount(1)

    // Dragging the phone preview to the right brings the left of the picture into view.
    const phone = section.getByTestId('settings-wallpaper-preview-phone')
    const box = (await phone.boundingBox())!
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
    await page.mouse.down()
    await page.mouse.move(box.x + box.width / 2 + 30, box.y + box.height / 2, { steps: 5 })
    await page.mouse.up()
    await expect.poll(async () => Number((await wallpaperLayer(page).evaluate((el) => getComputedStyle(el).backgroundPositionX)).replace('%', ''))).toBeLessThan(50)
    await expect(section.getByTestId('settings-wallpaper-recenter')).toBeVisible()

    await section.getByTestId('settings-wallpaper-fit-tile').click()
    await expect(section.getByTestId('settings-wallpaper-fit-tile')).toHaveAttribute('aria-checked', 'true')
    await expect(page.getByTestId('desktop-background')).toHaveAttribute('data-wallpaper-mode', 'tile')
    await expect(wallpaperLayer(page)).toHaveCSS('background-repeat', 'repeat')
    await expect(section.getByTestId('settings-wallpaper-tile-scale')).toBeAttached()

    // The account (the mock control-panel state) now holds the picture and its fit.
    await expect.poll(() => accountAppearance(page)).toMatchObject({
      wallpaper: {
        fit: 'tile',
        image: { ref: expect.stringMatching(/^obj:\/\/bucket\/user\/control-panel\/alice\/wallpapers\/[0-9a-f]{32}\.jpg$/), width: 1200, height: 800, name: 'sunset.jpg' },
      },
    })
    expect((await accountAppearance(page)).wallpaper.focusX).toBeLessThan(50)

    await page.reload()
    await expect(page.getByTestId('desktop-background')).toHaveAttribute('data-wallpaper-mode', 'tile')
    await expect(wallpaperLayer(page)).toHaveCSS('background-image', /^url\("blob:/)

    await page.getByTestId('desktop-app-settings').click()
    await page.getByTestId('window-settings').getByRole('button', { name: 'Appearance', exact: true }).click()
    await expect(page.getByTestId('settings-wallpaper-fit-tile')).toHaveAttribute('aria-checked', 'true')
    await page.getByTestId('settings-wallpaper-reset').click()
    await expect(page.getByTestId('settings-wallpaper')).toContainText('Using the default background')
    await expect(page.getByTestId('desktop-background')).not.toHaveAttribute('data-wallpaper-mode')
    await expect(page.locator('main[data-wallpaper]')).toHaveCount(0)
    await expect.poll(async () => (await accountAppearance(page)).wallpaper.image).toBeNull()
  })

  test('appearance follows the account: another device gets the theme, icon size and picture back', async ({ page }) => {
    const win = await openSettings(page, 'Appearance')
    await win.getByTestId('settings-theme').selectOption('dark')
    await win.getByTestId('settings-icon-size').selectOption('large')
    // From this device, through the picker's "This device".
    await win.getByTestId('settings-wallpaper-choose').click()
    await pickLocal(page, await landscapePicture(page))
    await win.getByTestId('settings-wallpaper-fit-fit').click()
    await expect.poll(() => accountAppearance(page)).toMatchObject({ theme: 'dark', iconSize: 'large', wallpaper: { fit: 'fit', image: { name: 'sunset.jpg' } } })

    // A new device: nothing of this page's own is left, only the account (which the mock keeps in the browser).
    await page.evaluate(() => {
      for (const key of Object.keys(localStorage)) {
        if (!key.startsWith('buckyos.mock.')) localStorage.removeItem(key)
      }
    })
    await page.reload()
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')
    await expect(page.getByTestId('desktop-background')).toHaveAttribute('data-wallpaper-mode', 'fit')
    await expect(wallpaperLayer(page)).toHaveCSS('background-image', /^url\("blob:/)
    await page.getByTestId('desktop-app-settings').click()
    await page.getByTestId('window-settings').getByRole('button', { name: 'Appearance', exact: true }).click()
    await expect(page.getByTestId('settings-icon-size')).toHaveValue('large')
    await expect(page.getByTestId('settings-wallpaper-status')).toContainText('sunset.jpg')

    // A change on one device reaches the account; the message preview stays a choice of this device.
    await page.getByTestId('settings-theme').selectOption('light')
    await expect.poll(async () => (await accountAppearance(page)).theme).toBe('light')
    expect(Object.keys(await accountAppearance(page))).not.toContain('messagePreview')
  })

  test('appearance: a file that is not a picture is refused and the background is kept', async ({ page }) => {
    const win = await openSettings(page, 'Appearance')
    // The picker only takes pictures: a text file is refused there and the picker stays open.
    await win.getByTestId('settings-wallpaper-choose').click()
    await pickLocal(page, { name: 'notes.txt', mimeType: 'text/plain', buffer: Buffer.from('hello') })
    await expect(selectDialog(page).getByTestId('select-notice')).toContainText('notes.txt')
    await pickLocal(page, { name: 'broken.png', mimeType: 'image/png', buffer: Buffer.from('not a png') })
    await expect(win.getByTestId('settings-wallpaper').getByRole('alert')).toContainText('could not be read')
    await expect(page.getByTestId('desktop-background')).not.toHaveAttribute('data-wallpaper-mode')
  })

  test('storage: what each app keeps; an uninstalled app leaves its data until it is deleted', async ({ page }) => {
    const win = await openSettings(page, 'Storage')
    const storage = win.getByTestId('settings-storage')
    await expect(storage.getByTestId('settings-storage-total')).toContainText('kept by 5 apps')
    const notes = storage.getByTestId('settings-storage-app-notes.alice.bns.did')
    await expect(notes).toContainText('Uninstalled')
    await expect(notes).toContainText('Documents')
    const collected = storage.getByTestId('settings-storage-app-msg-center')
    await expect(collected).toContainText('Collected')
    await expect(collected.getByRole('button', { name: 'Delete' })).toHaveCount(0)

    await notes.getByTestId('settings-storage-delete-user/notes.alice.bns.did/alice/notes').click()
    await expect(notes).toContainText('This cannot be undone')
    await notes.getByTestId('settings-storage-confirm-user/notes.alice.bns.did/alice/notes').click()
    await expect(storage.getByTestId('settings-storage-bucket-user/notes.alice.bns.did/alice/notes')).toHaveCount(0)
    await expect(storage.getByTestId('settings-storage-bucket-user/notes.alice.bns.did/alice/index')).toBeVisible()
  })

  test('agent runtimes: node policy, device issue and assignment, one Agent per device, revoke', async ({ page }) => {
    const win = await openSettings(page, 'Agent Runtimes')
    const runtimes = win.getByTestId('settings-agent-runtimes')
    const host = runtimes.getByTestId('settings-runtime-host')
    await expect(host).toContainText('Alice’s computer')
    await expect(host).toContainText('Open')
    await expect(host).toContainText('/home/alice · Read and write · ComputerUse: Screenshots and input · Standard environment')
    await expect(host).toContainText('Runs as the OS account chosen when the node was activated')

    // A second Agent of the same owner, to contend for the device.
    await page.evaluate(() => {
      const key = 'buckyos.mock.control-panel.v1'
      const state = JSON.parse(window.localStorage.getItem(key) ?? '{}')
      const coder = JSON.parse(JSON.stringify(state.agents[0]))
      Object.assign(coder, { agent_id: 'coder.alice', agent_did: 'did:bns:coder.alice', name: 'coder', profile: { display_name: 'Coder' }, idempotency_key: 'seed-coder' })
      Object.assign(coder.install, { agent_id: 'coder.alice', agent_did: 'did:bns:coder.alice' })
      state.agents.push(coder)
      window.localStorage.setItem(key, JSON.stringify(state))
    })

    await host.getByRole('button', { name: 'Edit', exact: true }).click()
    const edit = page.getByTestId('settings-runtime-edit')
    await expect(edit).toContainText('It cannot be changed here')
    await edit.getByLabel('Allowed working directories').fill('/home/alice\n/srv/data')
    await edit.getByLabel('Declared environment').click()
    await page.getByRole('option', { name: 'Extended environment' }).click()
    await edit.getByRole('button', { name: 'Save', exact: true }).click()
    await expect(host).toContainText('/home/alice, /srv/data · Read and write · ComputerUse: Screenshots and input · Extended environment')

    await runtimes.getByTestId('settings-add-device').click()
    const addDevice = page.getByRole('dialog', { name: 'Add device' })
    await expect(addDevice.getByRole('button', { name: 'Generate URL' })).toBeDisabled()
    await expect(addDevice).toContainText('Shell commands retain the current OS user')
    await addDevice.getByLabel('Device name').fill('Agent laptop')
    await addDevice.getByLabel('Allowed working directories').fill('/home/alice/work')
    await addDevice.getByLabel('Permissions').click()
    await page.getByRole('option', { name: 'Read files only' }).click()
    await addDevice.getByLabel('Assign to Agent (optional)').click()
    await page.getByRole('option', { name: 'BuckyOS Assistant' }).click()
    await addDevice.getByRole('button', { name: 'Generate URL' }).click()
    const issued = page.getByRole('dialog', { name: 'Start your device' })
    await expect(issued.getByLabel('Startup command')).toHaveValue(/start-buckyos-agent-daemon.sh/)
    await issued.getByRole('button', { name: 'Close', exact: true }).click()

    const devices = runtimes.getByTestId('settings-runtime-devices')
    const device = devices.locator('[data-testid^="settings-runtime-exec-"]')
    await expect(device).toContainText('Agent laptop')
    await expect(device).toContainText('/home/alice/work · Read only · ComputerUse: Disabled · Standard environment')
    await expect(device.getByTestId('settings-runtime-assignment-assistant.alice')).toContainText('On')
    const stored = await page.evaluate(() => JSON.parse(window.localStorage.getItem('buckyos.mock.control-panel.v1') ?? '{}'))
    expect(stored.executor_devices[0].profiles.default).toEqual({ workdir_roots: ['/home/alice/work'], ops: ['open_stream', 'describe', 'read', 'stat', 'query', 'cancel'], baseline: 'standard' })
    const runtimeId = `exec-${stored.executor_devices[0].executor_id}`
    expect(stored.agents[0].settings.runtimes[runtimeId]).toEqual({ enabled: true, target: { kind: 'executor', executor_id: stored.executor_devices[0].executor_id } })

    // The device is in use by the first Agent: the server refuses, the owner switches it over.
    await device.getByRole('button', { name: 'Assign to Agent', exact: true }).click()
    const assign = page.getByTestId('settings-runtime-assign')
    await assign.getByLabel('Agent', { exact: true }).click()
    await page.getByRole('option', { name: 'Coder' }).click()
    await assign.getByTestId('settings-runtime-assign-ok').click()
    await expect(assign).toContainText('This device is in use by BuckyOS Assistant')
    await assign.getByTestId('settings-runtime-assign-switch').click()
    await expect(assign).toHaveCount(0)
    await expect(device.getByTestId('settings-runtime-assignment-assistant.alice')).toContainText('Off')
    await expect(device.getByTestId('settings-runtime-assignment-coder.alice')).toContainText('On')

    await device.getByRole('button', { name: 'Unassign from BuckyOS Assistant' }).click()
    await page.getByTestId('settings-runtime-unassign-ok').click()
    await expect(device.getByTestId('settings-runtime-assignment-assistant.alice')).toHaveCount(0)

    await device.getByRole('button', { name: 'Revoke', exact: true }).click()
    const remove = page.getByTestId('settings-runtime-remove')
    await expect(remove).toContainText('It will be taken away from these Agents')
    await expect(remove).toContainText('Coder · using it now')
    await remove.getByTestId('settings-runtime-remove-ok').click()
    await expect(device).toContainText('Access revoked')
    await expect(device).toContainText('no Agent')
    await expect(device.getByRole('button', { name: 'Assign to Agent', exact: true })).toHaveCount(0)
    await device.getByRole('button', { name: 'Delete', exact: true }).click()
    await expect(page.getByTestId('settings-runtime-remove')).toContainText('No Agent is assigned to it')
    await page.getByTestId('settings-runtime-remove-ok').click()
    await expect(devices).toContainText('No device has been added yet')

    await host.getByRole('button', { name: 'Edit', exact: true }).click()
    await edit.getByRole('switch').click()
    await edit.getByRole('button', { name: 'Save', exact: true }).click()
    await expect(host).toContainText('Closed')
    await expect(host.getByRole('button', { name: 'Assign to Agent', exact: true })).toHaveCount(0)
  })

  test('cluster: devices with their hardware, certificates', async ({ page }) => {
    const win = await openSettings(page, 'Cluster Manager')
    const devices = win.getByTestId('settings-cluster-devices')
    await expect(devices.getByTestId('settings-device-ood1')).toContainText('This node')
    await expect(devices.getByTestId('settings-device-ood2')).toContainText('Online')
    await expect(devices.getByTestId('settings-device-laptop')).toContainText('Offline')

    await devices.getByTestId('settings-device-ood1').getByRole('button').first().click()
    const detail = devices.getByTestId('settings-device-detail-ood1')
    await expect(detail).toContainText('AMD Ryzen 7 5825U')
    await expect(detail).toContainText('Ubuntu 24.04.1 LTS')
    await expect(detail).toContainText('32 GB')

    await expect(devices.getByTestId('settings-add-device')).toHaveCount(0)

    await expect(win.getByTestId('settings-cluster-certificates')).toContainText('*.alice.web3.buckyos.ai')
    await expect(win.getByTestId('settings-cluster-certificates')).toContainText('ACME')
    await expect(win.getByTestId('settings-cluster-connectivity')).toContainText('sn.buckyos.ai')
  })

  test('privacy: public access, messaging, data and app access; no device permissions', async ({ page }) => {
    const win = await openSettings(page, 'Privacy')
    await expect(win.getByTestId('settings-privacy-public')).toContainText('Photo Gallery')
    await expect(win.getByTestId('settings-privacy-public')).toContainText('2026 家庭旅行计划')
    await expect(win.getByTestId('settings-privacy-messaging')).toContainText('2 blocked')
    await expect(win.getByTestId('settings-privacy-data')).toContainText('/srv/library/')
    await expect(win.getByTestId('settings-privacy-app-code-assistant@alice')).toContainText('/home/alice/projects')
    await expect(win.getByTestId('settings-privacy-agent-jarvis')).toContainText('telegram')
    await expect(win.getByText('Device & Capability Permissions')).toHaveCount(0)
  })

  test('developer: diagnostics, gateway config files and log download', async ({ page }) => {
    const win = await openSettings(page, 'Developer Mode')
    await expect(win.getByTestId('settings-dev-diagnostics')).toContainText('OOD nodes are reporting')
    await expect(win.getByTestId('settings-dev-diagnostics')).toContainText('TLS Certificate')

    const config = win.getByTestId('settings-dev-config')
    await config.getByRole('button', { name: 'node_gateway.json' }).click()
    await expect(config).toContainText('zone_tls')

    const download = page.waitForEvent('download')
    await win.getByTestId('settings-download-logs').click()
    expect((await download).suggestedFilename()).toBe('buckyos-logs-mock.txt')
    await expect(win.getByText('Coming soon')).toHaveCount(0)
  })
})

test.describe('settings on a phone', () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true })

  test('appearance: the picture covers the home screen and its labels turn light', async ({ page }) => {
    await page.goto('/?scenario=normal')
    // Launcher tiles open on a touch pointer release (see mobile.spec.ts).
    const settings = page.getByRole('button', { name: 'Settings' })
    const box = (await settings.boundingBox())!
    const at = { clientX: box.x + box.width / 2, clientY: box.y + box.height / 2, pointerId: 1, pointerType: 'touch', bubbles: true }
    await settings.dispatchEvent('pointerdown', at)
    await page.locator('body').dispatchEvent('pointerup', at)
    await page.getByRole('button', { name: 'Appearance' }).first().click()

    await expect(page.getByTestId('settings-wallpaper-preview-phone').locator('xpath=..')).toContainText('Phone · this screen')
    await putZoneFile(page, '/home/Pictures/sunset.jpg', (await landscapePicture(page)).buffer, 'image/jpeg')
    await page.getByTestId('settings-wallpaper-choose').tap()
    await pickZone(selectDialog(page), '/home/Pictures/sunset.jpg', true)
    await expect(page.getByTestId('settings-wallpaper-status')).toContainText('1200 × 800')
    await page.getByTestId('settings-wallpaper-fit-panorama').click()
    await page.getByRole('button', { name: 'Minimize' }).tap()

    await expect(page.getByTestId('desktop-background')).toHaveAttribute('data-wallpaper-mode', 'panorama')
    await expect(wallpaperLayer(page)).toHaveCSS('background-image', /^url\("blob:/)
    await expect(page.locator('.desktop-icon-label').first()).toHaveCSS('color', 'rgb(255, 255, 255)')
  })
})
