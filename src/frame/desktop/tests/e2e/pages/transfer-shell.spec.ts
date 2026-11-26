import { expect, test, type Frame, type Page } from '@playwright/test'
import { routeAppHost, routeAppSdk } from '../fixtures/desktop'

// Two Apps on their own origins (notes-fixture.localhost, reader-fixture.localhost) that take part in transfer.v1:
// each can offer the text of its textarea and shows what it received.
const FIXTURE = `<!doctype html><title>Transfer fixture</title><textarea id="text"></textarea><pre id="status"></pre><pre id="log">[]</pre><script type="module">
  import { AppFrameClient } from '/sdk/app-frame.mjs';
  const name = location.hostname.split('.')[0];
  document.querySelector('#text').value = 'Hello from ' + name;
  const log = [];
  const status = text => { document.querySelector('#status').textContent = text };
  const client = new AppFrameClient({ shellOrigin: 'http://127.0.0.1:4173', transfer: {
    onAvailable: () => status('available'),
    onProbe: probe => window.refuse ? null : { targetId: 'editor', action: 'Insert into ' + name, accept: probe.items.flatMap(item => {
      const format = ['application/vnd.buckyos.content-refs+json', 'text/plain'].find(f => item.formats.includes(f));
      return format ? [{ itemKey: item.itemKey, format, operation: 'copy' }] : [];
    }) },
    onDeliver: delivery => {
      log.push({ at: delivery.at, items: delivery.items.map(i => ({ format: i.format, value: i.value })) });
      document.querySelector('#log').textContent = JSON.stringify(log);
      return { status: 'done', items: delivery.items.map(i => ({ itemKey: i.itemKey, status: 'done' })) };
    },
    onRead: async read => {
      if (window.holdRead) await new Promise(() => {});
      return read.items.map(i => ({ ...i, value: i.format === 'application/vnd.buckyos.content-refs+json' ? window.refValue : document.querySelector('#text').value }));
    },
  } });
  window.offer = async () => { const r = await client.offerTransfer({ purpose: 'drag', items: [{ kind: 'text', formats: ['text/plain'], operations: ['copy'] }] }); status(r.ok ? 'offered' : 'refused ' + r.code); };
  window.offerRef = async ref => { window.refValue = ref; const r = await client.offerTransfer({ purpose: 'drag', items: [{ kind: 'zone-file', formats: ['application/vnd.buckyos.content-refs+json'], operations: ['copy'] }] }); status(r.ok ? 'offered' : 'refused ' + r.code); };
  window.openApp = request => client.openApp(request);
</script>`

const ref = { origin: 'zone', kind: 'file', source: { kind: 'cyfs-path', path: 'cyfs:///home/alice/notes.md' }, hints: { name: 'notes.md', mime: 'text/markdown' } }
const appFrame = (page: Page, name: 'notes' | 'reader'): Frame => page.frames().find((f) => f.url().startsWith(`http://${name}-fixture.`))!
const frameLog = async (page: Page, name: 'notes' | 'reader') => JSON.parse(await appFrame(page, name).locator('#log').textContent() ?? '[]') as Array<{ at: unknown; items: Array<{ format: string; value: unknown }> }>

async function windowId(page: Page, appId: string): Promise<string> {
  return page.evaluate(async (appId) => {
    const { desktopUIStore } = await import('/src/models/' + 'DesktopUIDataModel.ts')
    return desktopUIStore.getSnapshot().runtime.windows.find((w: { appId: string }) => w.appId === appId)!.id as string
  }, appId)
}

async function place(page: Page, appId: string, geometry: { x: number; y: number; width: number; height: number }, focus = false) {
  await page.evaluate(async ({ appId, geometry, focus }) => {
    const { desktopUIStore } = await import('/src/models/' + 'DesktopUIDataModel.ts')
    const id = desktopUIStore.getSnapshot().runtime.windows.find((w: { appId: string }) => w.appId === appId)!.id
    desktopUIStore.updateWindowGeometry(id, geometry)
    if (focus) desktopUIStore.focusWindow(id)
  }, { appId, geometry, focus })
}

async function openBuiltIn(page: Page, appId: string) {
  await page.evaluate(async (appId) => {
    const { desktopUIStore } = await import('/src/models/' + 'DesktopUIDataModel.ts')
    desktopUIStore.openAppWindow(appId)
  }, appId)
  await expect(page.getByTestId(`window-${appId}`)).toBeVisible()
}

async function focus(page: Page, appId: string) {
  await page.evaluate(async (appId) => {
    const { desktopUIStore } = await import('/src/models/' + 'DesktopUIDataModel.ts')
    desktopUIStore.focusWindow(desktopUIStore.getSnapshot().runtime.windows.find((w: { appId: string }) => w.appId === appId)!.id)
  }, appId)
}

/** Opens the notes and reader fixtures side by side, both connected. */
async function openFixtures(page: Page, query = '') {
  await page.setViewportSize({ width: 1440, height: 900 })
  await routeAppHost(page, /http:\/\/(notes|reader)-fixture\.localhost:4173\//, FIXTURE)
  await routeAppSdk(page)
  await page.goto(`/?scenario=normal&contentApps=1&open-app=notes-fixture${query}`)
  await expect.poll(() => appFrame(page, 'notes')?.locator('#status').textContent()).toBe('available')
  await appFrame(page, 'notes').evaluate(() => (window as unknown as { openApp: (r: object) => Promise<unknown> }).openApp({ app: 'reader-fixture' }))
  await expect.poll(() => appFrame(page, 'reader')?.locator('#status').textContent()).toBe('available')
  await place(page, 'notes-fixture', { x: 40, y: 60, width: 600, height: 420 })
  await place(page, 'reader-fixture', { x: 700, y: 60, width: 600, height: 420 })
}

const offerText = (page: Page, name: 'notes' | 'reader') => appFrame(page, name).evaluate(() => (window as unknown as { offer: () => Promise<void> }).offer())

/** A built-in window as a test-only transfer target that records what it got (Files and the others register their own). */
async function builtInTarget(page: Page, appId: string, options: { refuse?: boolean } = {}) {
  const id = await windowId(page, appId)
  await page.evaluate(async ({ id, refuse }) => {
    const { registerWindowTransferTarget } = (await import('/src/transfer/' + 'targets.ts') as typeof import('../../../src/transfer/targets'))
    const w = window as unknown as { builtInDelivered: unknown[]; builtInProbes: number }
    w.builtInDelivered = []; w.builtInProbes = 0
    registerWindowTransferTarget(id, {
      probe: (probe: { items: Array<{ itemKey: string; formats: string[] }> }) => {
        w.builtInProbes += 1
        return refuse ? null : { targetId: 'list', action: 'Add to the list', accept: probe.items.map((item) => ({ itemKey: item.itemKey, format: item.formats.includes('text/plain') ? 'text/plain' : item.formats[0], operation: 'copy' })) }
      },
      deliver: async (delivery: { items: Array<{ itemKey: string; value: unknown }> }) => {
        w.builtInDelivered.push(delivery)
        return { status: 'done', items: delivery.items.map((item) => ({ itemKey: item.itemKey, status: 'done' })) }
      },
    })
  }, { id, refuse: options.refuse ?? false })
}

async function dragHandle(page: Page, appId: string, to: { x: number; y: number }, options: { release?: boolean; steps?: number } = {}) {
  const handle = page.getByTestId(`window-${appId}`).getByTestId('transfer-handle')
  await expect(handle).toBeVisible()
  const box = (await handle.boundingBox())!
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.mouse.down()
  await page.mouse.move(to.x, to.y, { steps: options.steps ?? 12 })
  if (options.release !== false) await page.mouse.up()
}

const center = async (page: Page, appId: string) => {
  const box = (await page.getByTestId(`window-${appId}`).locator('[data-window-content]').boundingBox())!
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 }
}

test('iframe → iframe through the Shell handle: the preview names the target, the release delivers once', async ({ page }) => {
  await openFixtures(page)
  await offerText(page, 'notes')
  await focus(page, 'notes-fixture')
  // The handle belongs to the focused window only.
  await expect(page.getByTestId('window-reader-fixture').getByTestId('transfer-handle')).toHaveCount(0)
  const target = await center(page, 'reader-fixture')
  await dragHandle(page, 'notes-fixture', target, { release: false })
  const preview = page.getByTestId('transfer-preview')
  await expect(preview).toHaveAttribute('data-state', 'accepted')
  await expect(preview).toContainText('Reader Fixture: Insert into reader-fixture')
  await expect(preview).toContainText('1 selected in Notebook Fixture')
  await page.mouse.up()
  await expect(page.getByTestId('transfer-notice')).toContainText('Done in Reader Fixture: 1 delivered')
  await expect.poll(() => frameLog(page, 'reader')).toHaveLength(1)
  const [entry] = await frameLog(page, 'reader')
  expect(entry.items).toEqual([{ format: 'text/plain', value: 'Hello from notes-fixture' }])
  // The point is in the reader's own page.
  expect(entry.at).toEqual({ x: expect.any(Number), y: expect.any(Number) })
  expect(await frameLog(page, 'notes')).toEqual([])
})

test('tap the handle, then tap the destination window: one finger, no dragging', async ({ page }) => {
  await openFixtures(page)
  await offerText(page, 'reader')
  await focus(page, 'reader-fixture')
  await page.getByTestId('window-reader-fixture').getByTestId('transfer-handle').click()
  const chooser = page.getByTestId('transfer-choose')
  await expect(chooser).toContainText('Choose where to put 1 selected in Reader Fixture')
  // The list is the keyboard way to the same choice; here the window itself is tapped.
  await expect(chooser.getByRole('button', { name: /Notebook Fixture/ })).toBeVisible()
  const target = await center(page, 'notes-fixture')
  await page.mouse.click(target.x, target.y)
  await expect(page.getByTestId('transfer-notice')).toContainText('Done in Notebook Fixture: 1 delivered')
  await expect.poll(() => frameLog(page, 'notes')).toHaveLength(1)
  expect((await frameLog(page, 'notes'))[0].items).toEqual([{ format: 'text/plain', value: 'Hello from reader-fixture' }])
  await expect(chooser).toHaveCount(0)
})

test('keyboard: Enter on the handle, choose a window from the list, Esc cancels and restores focus', async ({ page }) => {
  await openFixtures(page)
  await offerText(page, 'notes')
  await focus(page, 'notes-fixture')
  const handle = page.getByTestId('window-notes-fixture').getByTestId('transfer-handle')
  await handle.focus()
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('transfer-choose')).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('transfer-choose')).toHaveCount(0)
  await expect(handle).toBeFocused()
  await page.keyboard.press('Enter')
  await page.getByTestId('transfer-choose').getByRole('button', { name: /Reader Fixture/ }).click()
  await expect.poll(() => frameLog(page, 'reader')).toHaveLength(1)
  expect((await frameLog(page, 'reader'))[0].at).toBe('focus')
})

test('Esc during a drag cancels it: nothing is read or delivered', async ({ page }) => {
  await openFixtures(page)
  await offerText(page, 'notes')
  await focus(page, 'notes-fixture')
  await dragHandle(page, 'notes-fixture', await center(page, 'reader-fixture'), { release: false })
  await expect(page.getByTestId('transfer-preview')).toHaveAttribute('data-state', 'accepted')
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('transfer-preview')).toHaveCount(0)
  await page.mouse.up()
  await page.waitForTimeout(300)
  expect(await frameLog(page, 'reader')).toEqual([])
})

test('a refusing target shows the disabled state and the release delivers nothing', async ({ page }) => {
  await openFixtures(page)
  await offerText(page, 'notes')
  await appFrame(page, 'reader').evaluate(() => { (window as unknown as { refuse: boolean }).refuse = true })
  await focus(page, 'notes-fixture')
  await dragHandle(page, 'notes-fixture', await center(page, 'reader-fixture'), { release: false })
  await expect(page.getByTestId('transfer-preview')).toHaveAttribute('data-state', 'refused')
  await expect(page.getByTestId('transfer-preview')).toContainText('Reader Fixture can’t take this here')
  await page.mouse.up()
  await expect(page.getByTestId('transfer-notice')).toContainText('Nothing was delivered')
  expect(await frameLog(page, 'reader')).toEqual([])
})

test('the source reloading before its data is read makes the operation stale: nothing is delivered', async ({ page }) => {
  await openFixtures(page)
  await offerText(page, 'notes')
  await appFrame(page, 'notes').evaluate(() => { (window as unknown as { holdRead: boolean }).holdRead = true })
  await focus(page, 'notes-fixture')
  await dragHandle(page, 'notes-fixture', await center(page, 'reader-fixture'))
  // The read is waiting on the notes document; the same frame loads again (same origin, same window id).
  await appFrame(page, 'notes').evaluate(() => location.reload())
  await expect(page.getByTestId('transfer-notice')).toContainText('The selection changed', { timeout: 15000 })
  await expect.poll(() => appFrame(page, 'notes')?.locator('#status').textContent()).toBe('available')
  // The new document has offered nothing: no handle, and the reader never got anything.
  await focus(page, 'notes-fixture')
  await expect(page.getByTestId('window-notes-fixture').getByTestId('transfer-handle')).toHaveCount(0)
  expect(await frameLog(page, 'reader')).toEqual([])
})

test('a reload while choosing the destination withdraws the selection', async ({ page }) => {
  await openFixtures(page)
  await offerText(page, 'notes')
  await focus(page, 'notes-fixture')
  await page.getByTestId('window-notes-fixture').getByTestId('transfer-handle').click()
  await expect(page.getByTestId('transfer-choose')).toBeVisible()
  await appFrame(page, 'notes').evaluate(() => location.reload())
  await expect(page.getByTestId('transfer-choose')).toHaveCount(0)
  await expect(page.getByTestId('transfer-notice')).toContainText('The selection changed')
})

test('a covered window never receives: the topmost window under the pointer is the only target', async ({ page }) => {
  await openFixtures(page)
  await openBuiltIn(page, 'settings')
  await builtInTarget(page, 'settings', { refuse: true })
  // Settings covers the middle of the reader window.
  await place(page, 'settings', { x: 800, y: 120, width: 400, height: 300 }, true)
  await offerText(page, 'notes')
  await focus(page, 'notes-fixture')
  const covered = await center(page, 'settings')
  await dragHandle(page, 'notes-fixture', covered, { release: false })
  await expect(page.getByTestId('transfer-preview')).toHaveAttribute('data-state', 'refused')
  await expect(page.getByTestId('transfer-preview')).toContainText('Settings can’t take this here')
  await page.mouse.up()
  await page.waitForTimeout(300)
  expect(await frameLog(page, 'reader')).toEqual([])
  expect(await page.evaluate(() => (window as unknown as { builtInDelivered: unknown[] }).builtInDelivered)).toEqual([])
})

test('system App → iframe and iframe → system App', async ({ page }) => {
  await openFixtures(page)
  await openBuiltIn(page, 'settings')
  await place(page, 'settings', { x: 380, y: 500, width: 520, height: 360 }, true)
  await builtInTarget(page, 'settings')
  // A built-in source registers its selection for the Shell's handle.
  const settings = await windowId(page, 'settings')
  await page.evaluate(async ({ settings, ref }) => {
    const { offerWindowSelection } = (await import('/src/transfer/' + 'shell.ts') as typeof import('../../../src/transfer/shell'))
    offerWindowSelection('settings', settings, { items: [{ kind: 'zone-file', formats: { 'application/vnd.buckyos.content-refs+json': ref, 'text/plain': 'notes.md' } }] })
  }, { settings, ref })
  await dragHandle(page, 'settings', await center(page, 'reader-fixture'))
  await expect.poll(() => frameLog(page, 'reader')).toHaveLength(1)
  expect((await frameLog(page, 'reader'))[0].items).toEqual([{ format: 'application/vnd.buckyos.content-refs+json', value: ref }])

  await offerText(page, 'notes')
  await focus(page, 'notes-fixture')
  await dragHandle(page, 'notes-fixture', await center(page, 'settings'))
  await expect(page.getByTestId('transfer-notice')).toContainText('Done in Settings: 1 delivered')
  const delivered = await page.evaluate(() => (window as unknown as { builtInDelivered: Array<{ items: Array<{ format: string; value: unknown }>; at: unknown }> }).builtInDelivered)
  expect(delivered).toHaveLength(1)
  expect(delivered[0].items).toEqual([{ itemKey: '0', format: 'text/plain', value: 'Hello from notes-fixture' }])
})

test('Paste desktop object… shows the pending object and its destination, then delivers at the focus', async ({ page }) => {
  await openFixtures(page)
  await offerText(page, 'notes')
  await focus(page, 'reader-fixture')
  await page.getByTestId('window-reader-fixture').getByTestId('window-menu').click()
  await page.getByRole('menuitem', { name: 'Paste desktop object…' }).click()
  const dialog = page.getByRole('dialog', { name: 'Paste desktop object' })
  await expect(dialog).toContainText('From Notebook Fixture: 1 selected')
  await expect(dialog).toContainText('Into Reader Fixture')
  await expect(dialog).toContainText('Insert into reader-fixture')
  await dialog.getByRole('button', { name: 'Paste' }).click()
  await expect.poll(() => frameLog(page, 'reader')).toHaveLength(1)
  const [entry] = await frameLog(page, 'reader')
  expect(entry.at).toBe('focus')
  expect(entry.items).toEqual([{ format: 'text/plain', value: 'Hello from notes-fixture' }])
})

test.describe('with a touch screen', () => {
  test.use({ hasTouch: true })
  test('one finger: tap the handle, then tap the window that receives', async ({ page }) => {
    await openFixtures(page)
    await offerText(page, 'notes')
    await focus(page, 'notes-fixture')
    const handle = (await page.getByTestId('window-notes-fixture').getByTestId('transfer-handle').boundingBox())!
    await page.touchscreen.tap(handle.x + handle.width / 2, handle.y + handle.height / 2)
    await expect(page.getByTestId('transfer-choose')).toBeVisible()
    const target = await center(page, 'reader-fixture')
    await page.touchscreen.tap(target.x, target.y)
    await expect.poll(() => frameLog(page, 'reader')).toHaveLength(1)
    expect((await frameLog(page, 'reader'))[0].items).toEqual([{ format: 'text/plain', value: 'Hello from notes-fixture' }])
  })
})

test('phone: Send to another app… reads the selection, switches to the chosen App and delivers after confirming', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await routeAppHost(page, /http:\/\/(notes|reader)-fixture\.localhost:4173\//, FIXTURE)
  await routeAppSdk(page)
  await page.goto('/?scenario=normal&contentApps=1&open-app=notes-fixture')
  await expect.poll(() => appFrame(page, 'notes')?.locator('#status').textContent()).toBe('available')
  // Open the reader, then come back: a phone keeps one App loaded at a time.
  // The notes frame goes away as the reader opens over it: the request is not awaited there.
  await appFrame(page, 'notes').evaluate(() => { void (window as unknown as { openApp: (r: object) => Promise<unknown> }).openApp({ app: 'reader-fixture' }) })
  await expect.poll(() => appFrame(page, 'reader')?.locator('#status').textContent()).toBe('available')
  await page.evaluate(async () => {
    const { desktopUIStore } = await import('/src/models/' + 'DesktopUIDataModel.ts')
    const notes = desktopUIStore.getSnapshot().runtime.windows.find((w: { appId: string }) => w.appId === 'notes-fixture')!
    desktopUIStore.openAppWindow('notes-fixture', { windowId: notes.id })
  })
  await expect.poll(() => appFrame(page, 'notes')?.locator('#status').textContent()).toBe('available')
  await offerText(page, 'notes')
  await page.getByRole('button', { name: 'App menu' }).click()
  await page.getByRole('menuitem', { name: 'Send to another app…' }).click()
  const chooser = page.getByTestId('transfer-choose')
  await expect(chooser).toContainText('Choose the app to send it to')
  await chooser.getByRole('button', { name: /Reader Fixture/ }).click()
  const dialog = page.getByRole('dialog', { name: 'Paste desktop object' })
  await expect(dialog).toContainText('Insert into reader-fixture', { timeout: 15000 })
  await dialog.getByRole('button', { name: 'Paste' }).click()
  await expect.poll(() => frameLog(page, 'reader')).toHaveLength(1)
  expect((await frameLog(page, 'reader'))[0]).toEqual({ at: 'focus', items: [{ format: 'text/plain', value: 'Hello from notes-fixture' }] })
})

test('router: no offer replaces another, and a refused or late final answer never delivers anywhere', async ({ page }) => {
  await page.goto('/?scenario=normal')
  const result = await page.evaluate(async () => {
    const { desktopTransfers } = (await import('/src/transfer/' + 'records.ts') as typeof import('../../../src/transfer/records'))
    const { registerWindowTransferTarget } = (await import('/src/transfer/' + 'targets.ts') as typeof import('../../../src/transfer/targets'))
    const shell = (await import('/src/transfer/' + 'shell.ts') as typeof import('../../../src/transfer/shell'))
    const delivered: string[] = []
    const item = { kind: 'text' as const, formats: { 'text/plain': 'hi' } }
    const input = { purpose: 'drag' as const, source: { app: 'src', windowId: 'win-src' }, itemKeys: ['0'], items: [item], formats: {}, operations: ['copy' as const] }
    const first = desktopTransfers.offerWithId('offer-collision-000001', input)
    const second = desktopTransfers.offerWithId('offer-collision-000001', input)
    const accept = { targetId: 't', action: 'Take', accept: [{ itemKey: '0', format: 'text/plain', operation: 'copy' as const }] }
    const done = (d: { items: Array<{ itemKey: string }> }) => ({ status: 'done' as const, items: d.items.map((i) => ({ itemKey: i.itemKey, status: 'done' as const })) })
    let laterProbes = 0
    registerWindowTransferTarget('win-a', { probe: () => accept, deliver: async (d) => { delivered.push('a'); return done(d) } })
    registerWindowTransferTarget('win-b', { probe: () => (++laterProbes === 1 ? accept : null), deliver: async (d) => { delivered.push('b'); return done(d) } })
    registerWindowTransferTarget('win-slow', { probe: () => new Promise(() => {}), deliver: async (d) => { delivered.push('slow'); return done(d) } })
    const op = shell.beginTransfer(first!.offerId)!
    const hoverA = await shell.probeTransfer(op, 'win-a', { x: 1, y: 1 })
    const hoverB = await shell.probeTransfer(op, 'win-b', { x: 1, y: 1 })
    const refused = await shell.deliverTransfer(op, 'win-b', { x: 1, y: 1 })
    const op2 = shell.beginTransfer(first!.offerId)!
    await shell.probeTransfer(op2, 'win-a', 'focus')
    const late = await shell.deliverTransfer(op2, 'win-slow', 'focus')
    return { first: !!first, second: !!second, hoverA: hoverA.ok, hoverB: hoverB.ok, refused, late, delivered }
  })
  expect(result.first).toBe(true)
  expect(result.second).toBe(false)
  expect([result.hoverA, result.hoverB]).toEqual([true, true])
  expect(result.refused).toMatchObject({ status: 'rejected', windowId: 'win-b', reason: 'none' })
  expect(result.late).toMatchObject({ status: 'rejected', windowId: 'win-slow', reason: 'timeout' })
  expect(result.delivered).toEqual([])
})

test('router: a cut gives up only the items that moved; a delivery without a receipt is checked with its own operation id', async ({ page }) => {
  test.setTimeout(60000)
  await page.goto('/?scenario=normal')
  const result = await page.evaluate(async () => {
    const { desktopTransfers } = (await import('/src/transfer/' + 'records.ts') as typeof import('../../../src/transfer/records'))
    const { registerWindowTransferTarget } = (await import('/src/transfer/' + 'targets.ts') as typeof import('../../../src/transfer/targets'))
    const shell = (await import('/src/transfer/' + 'shell.ts') as typeof import('../../../src/transfer/shell'))
    const files = ['a', 'b'].map((name) => ({ kind: 'zone-file' as const, formats: { 'text/plain': name } }))
    const cut = desktopTransfers.offer({ purpose: 'cut', source: { app: 'files', windowId: 'win-files' }, itemKeys: ['/a', '/b'], items: files, formats: {}, operations: ['move'] })
    registerWindowTransferTarget('win-move', {
      probe: (p) => ({ targetId: 'dir', action: 'Move here', accept: p.items.map((i) => ({ itemKey: i.itemKey, format: 'text/plain', operation: 'move' as const })) }),
      deliver: async (d) => ({ status: 'done', items: [{ itemKey: d.items[0].itemKey, status: 'done' }, { itemKey: d.items[1].itemKey, status: 'failed', error: 'exists' }] }),
    })
    const moved = await shell.deliverTransfer(shell.beginTransfer(cut.offerId)!, 'win-move', 'focus')
    const left = desktopTransfers.probeItems(cut.offerId)?.map((i) => i.itemKey)

    const copy = desktopTransfers.offer({ purpose: 'copy', source: { app: 'notes', windowId: 'win-notes' }, itemKeys: ['0'], items: [{ kind: 'text', formats: { 'text/plain': 'x' } }], formats: {}, operations: ['copy'] })
    const deliveries: string[] = []
    const statuses: string[] = []
    registerWindowTransferTarget('win-quiet', {
      probe: () => ({ targetId: 'draft', action: 'Add', accept: [{ itemKey: '0', format: 'text/plain', operation: 'copy' }] }),
      deliver: (d) => { deliveries.push(d.operationId); return new Promise(() => {}) },
      status: async (operationId) => { statuses.push(operationId); return { status: 'accepted', items: [{ itemKey: '0', status: 'accepted' }] } },
    })
    const op = shell.beginTransfer(copy.offerId)!
    const pending = await shell.deliverTransfer(op, 'win-quiet', 'focus')
    const checked = await shell.retryTransfer(op)
    return { moved, left, pending: pending.status, checked, sameId: statuses[0] === op.operationId && deliveries[0] === op.operationId, deliveries: deliveries.length }
  })
  expect(result.moved).toMatchObject({ status: 'done', items: [{ itemKey: '0', status: 'done' }, { itemKey: '1', status: 'failed' }] })
  expect(result.left).toEqual(['1'])
  expect(result.pending).toBe('pending')
  expect(result.checked).toMatchObject({ status: 'accepted', items: [{ itemKey: '0', status: 'accepted' }] })
  expect(result.sameId).toBe(true)
  expect(result.deliveries).toBe(1)
})

test('a target that reloaded keeps its window and is bound again; a fast release still goes only to the final window', async ({ page }) => {
  await openFixtures(page)
  const before = await windowId(page, 'reader-fixture')
  await appFrame(page, 'reader').evaluate(() => location.reload())
  await expect.poll(() => appFrame(page, 'reader')?.locator('#status').textContent()).toBe('available')
  expect(await windowId(page, 'reader-fixture')).toBe(before)
  await offerText(page, 'notes')
  await focus(page, 'notes-fixture')
  await dragHandle(page, 'notes-fixture', await center(page, 'reader-fixture'), { steps: 2 })
  await expect.poll(() => frameLog(page, 'reader')).toHaveLength(1)
  expect((await frameLog(page, 'reader'))[0].items).toEqual([{ format: 'text/plain', value: 'Hello from notes-fixture' }])
})

/** Opens a Files window on a folder and places it. */
async function openFiles(page: Page, path: string, geometry: { x: number; y: number; width: number; height: number }) {
  await page.evaluate(async ({ path, geometry }) => {
    const { desktopUIStore } = await import('/src/models/' + 'DesktopUIDataModel.ts')
    const { filesOpenLaunch } = await import('/src/app/filebrowser/' + 'launch.ts')
    desktopUIStore.openAppWindow('files', { multi: true, mode: 'new', launch: filesOpenLaunch(path) })
    const windows = desktopUIStore.getSnapshot().runtime.windows.filter((w: { appId: string }) => w.appId === 'files')
    desktopUIStore.updateWindowGeometry(windows[windows.length - 1].id, geometry)
  }, { path, geometry })
  const win = page.getByTestId('window-files').last()
  await expect(win.locator('[data-pane="left"]')).toHaveAttribute('data-list-status', 'ready')
  return win
}

test('Files → iframe and iframe → Files through the Shell handle, with the real Files target', async ({ page }) => {
  await openFixtures(page, '&fbData=mock')
  // Files' selection is offered to the Shell: its handle carries the Zone reference into the reader App.
  const files = await openFiles(page, '/home/Documents', { x: 40, y: 500, width: 620, height: 380 })
  await files.getByRole('cell', { name: 'Kyoto Trip Plan.md', exact: true }).click()
  await dragHandle(page, 'files', await center(page, 'reader-fixture'))
  await expect.poll(() => frameLog(page, 'reader')).toHaveLength(1)
  const [delivered] = await frameLog(page, 'reader')
  expect(delivered.items).toEqual([{ format: 'application/vnd.buckyos.content-refs+json', value: expect.objectContaining({ origin: 'zone', kind: 'file', source: { kind: 'cyfs-path', path: 'cyfs:///home/Documents/Kyoto Trip Plan.md' } }) }])

  // A Zone file the notes App offers is copied by Files into the folder under the pointer.
  const pictures = await openFiles(page, '/home/Pictures', { x: 700, y: 500, width: 620, height: 380 })
  await appFrame(page, 'notes').evaluate((ref) => (window as unknown as { offerRef: (r: unknown) => Promise<void> }).offerRef(ref), { origin: 'zone', kind: 'file', source: { kind: 'cyfs-path', path: 'cyfs:///home/Documents/Kyoto Trip Plan.md' }, hints: { name: 'Kyoto Trip Plan.md' } })
  await expect.poll(() => appFrame(page, 'notes').locator('#status').textContent()).toBe('offered')
  await focus(page, 'notes-fixture')
  const box = (await pictures.locator('[data-pane="left"]').boundingBox())!
  await dragHandle(page, 'notes-fixture', { x: box.x + box.width / 2, y: box.y + box.height / 2 })
  await expect(pictures.getByRole('cell', { name: 'Kyoto Trip Plan.md', exact: true })).toBeVisible()
  expect(await page.evaluate(async () => !!(await import('/src/app/filebrowser/mock/' + 'data.ts')).mockEntryByPath('/home/Pictures/Kyoto Trip Plan.md'))).toBe(true)
})
