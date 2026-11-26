import { CONTENT_REFS_FORMAT, contentRefsRepresentations } from 'buckyos/transfer'
import type { ZoneContent } from 'buckyos/content'
import { createNamedSession } from '../fixtures/messagehub'
import { expect, test, type Page } from '@playwright/test'

/**
 * Chat as a receiver and a source of Zone content (剪贴板规范 §2.2, §7 cases 3 and 7) on the mock runtime: Zone files
 * dropped or pasted into a conversation join the attachment draft and are never sent by themselves; an address pasted
 * as text stays text; an attachment dragged out of the history carries its object reference.
 */

declare global {
  interface Window { __messageHubMock: import('../../../src/app/messagehub/mock/store').MessageHubMockStore }
}

const SELF = 'did:buckyos:user:self'
const CODER = 'did:buckyos:agent:codeassistant'
const ALICE = 'did:buckyos:person:alice'
const OWN = { viewerDid: SELF, ownerDid: SELF, mode: 'self' as const }

const zoneFile = (path: string, size?: number): ZoneContent => ({ origin: 'zone', kind: 'file', source: { kind: 'cyfs-path', path: `cyfs://${path}` }, hints: { name: path.split('/').pop()!, ...(size !== undefined ? { size } : {}) } })
const zoneFolder = (path: string): ZoneContent => ({ origin: 'zone', kind: 'folder', source: { kind: 'cyfs-path', path: `cyfs://${path}` }, hints: { name: path.split('/').pop()! } })

async function openSession(page: Page, title: string) {
  await page.goto(`/messagehub?entityId=${encodeURIComponent(CODER)}`)
  await expect(page.getByRole('button', { name: 'Sessions', exact: true })).toBeVisible()
  await createNamedSession(page, title, { setDefault: true })
  await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-raw-count', '0')
  return page.evaluate(({ context, entity }) => window.__messageHubMock.sessions(context, entity)[0].id, { context: OWN, entity: CODER })
}

/**
 * Dispatches a drag over and a drop on `selector` with a synthesized DataTransfer carrying `data`; with `overlay`, the
 * drop waits until the conversation names the action it offers.
 */
async function dropData(page: Page, selector: string, data: Record<string, string>, options: { overlay?: boolean } = {}) {
  const target = page.locator(selector).first()
  const accepted = await target.evaluate((element, data) => {
    const transfer = new DataTransfer()
    for (const [type, value] of Object.entries(data)) transfer.setData(type, value)
    ;(window as unknown as { __transfer: DataTransfer }).__transfer = transfer
    const over = new DragEvent('dragover', { bubbles: true, cancelable: true, dataTransfer: transfer })
    element.dispatchEvent(new DragEvent('dragenter', { bubbles: true, cancelable: true, dataTransfer: transfer }))
    element.dispatchEvent(over)
    return over.defaultPrevented
  }, data)
  if (options.overlay !== false && accepted) await expect(page.getByTestId('conversation-drop-action')).toHaveText('Add to attachment draft')
  const dropped = await target.evaluate((element) => {
    const drop = new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer: (window as unknown as { __transfer: DataTransfer }).__transfer })
    element.dispatchEvent(drop)
    return drop.defaultPrevented
  })
  return { accepted, dropped }
}

test('Zone files and a folder dropped on a conversation join the draft and nothing is sent', async ({ page }) => {
  const id = await openSession(page, 'Drop from Files')
  const composer = page.getByTestId('message-composer')
  const representations = contentRefsRepresentations([zoneFile('/home/Documents/Kyoto Trip Plan.md', 18_432), zoneFolder('/home/Pictures/sketches'), zoneFile('/home/Documents/missing.txt')])
  // Hover takes the types alone and names the action; the drop is claimed.
  const result = await dropData(page, '[data-testid="conversation-history"]', representations)
  expect(result).toEqual({ accepted: true, dropped: true })
  await expect(page.getByTestId('conversation-drop-action')).toHaveCount(0)

  await expect(composer.getByRole('button', { name: 'Remove Kyoto Trip Plan.md', exact: true })).toBeVisible()
  await expect(composer.getByRole('button', { name: 'Remove sketches', exact: true })).toBeVisible()
  // One failure, named on its own; the others are in the draft.
  await expect(composer.getByTestId('zone-attach-failed')).toContainText('Could not add missing.txt: it no longer exists')
  await expect(composer.getByTestId('zone-attach-pending')).toHaveCount(0)
  await expect(page.locator('textarea')).toHaveValue('')
  await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-raw-count', '0')
  expect(await page.evaluate(({ context, id }) => window.__messageHubMock.reader(context, id).readRange(0, 5), { context: OWN, id })).toHaveLength(0)

  // The same item again is not added twice.
  await dropData(page, '[data-testid="message-composer"]', contentRefsRepresentations([zoneFile('/home/Documents/Kyoto Trip Plan.md')]))
  await expect(composer.getByRole('button', { name: 'Remove Kyoto Trip Plan.md', exact: true })).toHaveCount(1)

  // Sending stays the user's decision: the bytes read from the Zone go out as ordinary attachments.
  await page.locator('textarea').press('Enter')
  await expect.poll(() => page.evaluate(async ({ context, id }) => (await window.__messageHubMock.reader(context, id).readRange(0, 1)).map(m => [m.ui_local_id ?? 'stored', (m.content.refs ?? []).map(r => r.label)]), { context: OWN, id }), { timeout: 15_000 })
    .toEqual([['stored', ['Kyoto Trip Plan.md', 'sketches']]])
})

test('a Zone file larger than the attachment limit is refused before it is read', async ({ page }) => {
  await openSession(page, 'Too large')
  const composer = page.getByTestId('message-composer')
  await dropData(page, '[data-testid="conversation-history"]', contentRefsRepresentations([zoneFile('/home/Downloads/market-report-2026.pdf', 80 * 1024 * 1024)]))
  await expect(composer.getByTestId('zone-attach-failed')).toContainText('Could not add market-report-2026.pdf: it is larger than 64 MB')
  await expect(composer.getByRole('button', { name: /^Remove / })).toHaveCount(0)
})

test('Zone files pasted into the input become attachments, their names are not typed in', async ({ page }) => {
  await openSession(page, 'Paste from Files')
  const textarea = page.locator('textarea')
  await textarea.fill('See ')
  const prevented = await textarea.evaluate((element, data) => {
    const transfer = new DataTransfer()
    for (const [type, value] of Object.entries(data)) transfer.setData(type, value)
    const paste = new ClipboardEvent('paste', { bubbles: true, cancelable: true, clipboardData: transfer })
    element.dispatchEvent(paste)
    return paste.defaultPrevented
  }, contentRefsRepresentations([zoneFile('/home/Documents/Kyoto Trip Plan.md')]))
  expect(prevented).toBe(true)
  await expect(page.getByTestId('message-composer').getByRole('button', { name: 'Remove Kyoto Trip Plan.md', exact: true })).toBeVisible()
  await expect(textarea).toHaveValue('See ')
  await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-raw-count', '0')
})

test('through the system clipboard: a copied Zone file arrives by its HTML links, a cyfs:// address stays text', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  await openSession(page, 'System clipboard')
  const textarea = page.locator('textarea')
  const composer = page.getByTestId('message-composer')

  // What a toolbar Copy writes; a paste event cannot read the custom format, the marked links carry the reference.
  const written = contentRefsRepresentations([zoneFile('/home/Documents/Kyoto Trip Plan.md')])
  expect(written[CONTENT_REFS_FORMAT]).toBeTruthy()
  await page.evaluate(async ({ text, html }) => {
    await navigator.clipboard.write([new ClipboardItem({ 'text/plain': new Blob([text], { type: 'text/plain' }), 'text/html': new Blob([html], { type: 'text/html' }) })])
  }, { text: written['text/plain'], html: written['text/html'] })
  await textarea.focus()
  await page.keyboard.press('ControlOrMeta+V')
  await expect(composer.getByRole('button', { name: 'Remove Kyoto Trip Plan.md', exact: true })).toBeVisible()
  await expect(textarea).toHaveValue('')

  // An address is text in a text input (§7 case 7): inserted, nothing read or attached.
  await page.evaluate(() => navigator.clipboard.writeText('cyfs:///home/Documents/2026%20Q1%20Review.docx'))
  await textarea.focus()
  await page.keyboard.press('ControlOrMeta+V')
  await expect(textarea).toHaveValue('cyfs:///home/Documents/2026%20Q1%20Review.docx')
  await expect(composer.getByRole('button', { name: /^Remove / })).toHaveCount(1)
  await expect(composer.getByTestId('zone-attach-pending')).toHaveCount(0)
})

test('a text dragged onto the input is left to the input; nothing joins the draft', async ({ page }) => {
  await openSession(page, 'Text drag')
  const result = await dropData(page, 'textarea', { 'text/plain': 'hello', 'text/html': '<b>hello</b>' }, { overlay: false })
  expect(result).toEqual({ accepted: false, dropped: false })
  await expect(page.getByTestId('conversation-drop-action')).toHaveCount(0)
  await expect(page.getByTestId('message-composer').getByRole('button', { name: /^Remove / })).toHaveCount(0)
})

test('an attachment dragged out of the history carries its object reference and readable names', async ({ page }) => {
  await page.route('https://upload.wikimedia.org/**', route => route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"/>' }))
  await page.goto(`/messagehub?entityId=${encodeURIComponent(ALICE)}`)
  const card = page.getByTestId('conversation-history').getByTestId('attachment-ready').filter({ hasText: 'design-brief.pdf' })
  await expect(card).toBeVisible({ timeout: 15_000 })
  await expect(card).toHaveAttribute('draggable', 'true')
  const written = await card.evaluate((element) => {
    const transfer = new DataTransfer()
    const source = element.querySelector('button')!
    source.dispatchEvent(new DragEvent('dragstart', { bubbles: true, cancelable: true, dataTransfer: transfer }))
    // On its way out, it is not offered back to the draft of the conversation it comes from; after it, a drag is.
    const history = document.querySelector('[data-testid="conversation-history"]')!
    const over = () => { const event = new DragEvent('dragover', { bubbles: true, cancelable: true, dataTransfer: transfer }); history.dispatchEvent(event); return event.defaultPrevented }
    const duringOwnDrag = over()
    source.dispatchEvent(new DragEvent('dragend', { bubbles: true, dataTransfer: transfer }))
    const afterwards = over()
    return { types: [...transfer.types], refs: transfer.getData('application/vnd.buckyos.content-refs+json'), text: transfer.getData('text/plain'), uris: transfer.getData('text/uri-list'), duringOwnDrag, afterwards }
  })
  expect(written).toMatchObject({ duringOwnDrag: false, afterwards: true })
  expect(written.types).toEqual(expect.arrayContaining([CONTENT_REFS_FORMAT, 'text/plain', 'text/uri-list', 'text/html']))
  expect(JSON.parse(written.refs)).toEqual({ v: 1, items: [{ origin: 'zone', kind: 'file', source: { kind: 'object-id', objectId: 'cyfile:mock-doc-brief' }, hints: expect.objectContaining({ name: 'design-brief.pdf', mime: 'application/pdf' }) }] })
  // People read a name, never JSON or an internal id; the address is in the URI list.
  expect(written.text).toBe('design-brief.pdf')
  expect(written.uris).toBe('obj://cyfile:mock-doc-brief')
})

test('desktop: a MessageHub window takes what the Shell hands it into the draft, once per operation', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-messagehub').click()
  const hub = page.getByTestId('window-messagehub')
  await hub.getByText('Alice Chen', { exact: true }).first().click()
  await expect(hub.getByTestId('message-composer')).toBeVisible({ timeout: 15_000 })
  const items = [zoneFile('/home/Documents/Kyoto Trip Plan.md'), zoneFile('/home/Documents/missing.txt')]
  const outcome = await page.evaluate(async (items) => {
    const desktop = '/src/models/DesktopUIDataModel.ts'
    const targets = '/src/transfer/targets.ts'
    const { desktopUIStore } = await import(desktop)
    const { windowTransferTarget } = await import(targets)
    const windowId = (desktopUIStore.getSnapshot().runtime.windows as Array<{ id: string; appId: string }>).find((w) => w.appId === 'messagehub')!.id
    const target = windowTransferTarget(windowId)!
    const probeItems = [
      { itemKey: '0', kind: 'zone-file', formats: ['application/vnd.buckyos.content-refs+json'], operations: ['copy', 'move'] },
      { itemKey: '1', kind: 'zone-file', formats: ['application/vnd.buckyos.content-refs+json'], operations: ['copy'] },
      { itemKey: '2', kind: 'local-file', formats: ['files'], operations: ['copy'] },
      { itemKey: '3', kind: 'object', formats: ['application/vnd.buckyos-aiws.clip+json'], operations: ['copy'] },
      { itemKey: '4', kind: 'zone-file', formats: ['application/vnd.buckyos.content-refs+json'], operations: ['move'] },
    ]
    const answer = await target.probe({ operationId: 'op-1', seq: 1, items: probeItems, at: 'focus' })
    const delivery = {
      operationId: 'op-1', targetId: answer!.targetId, operation: 'copy', at: 'focus',
      items: [
        { itemKey: '0', format: 'application/vnd.buckyos.content-refs+json', value: items[0] },
        { itemKey: '1', format: 'application/vnd.buckyos.content-refs+json', value: items[1] },
        { itemKey: '2', format: 'files', value: new File(['from this device'], 'note.txt', { type: 'text/plain' }) },
      ],
    }
    const first = await target.deliver(delivery)
    const again = await target.deliver(delivery)
    return { answer, first, same: first === again }
  }, items)
  expect(outcome.answer).toMatchObject({ action: 'Add to attachment draft', accept: [
    { itemKey: '0', format: CONTENT_REFS_FORMAT, operation: 'copy' },
    { itemKey: '1', format: CONTENT_REFS_FORMAT, operation: 'copy' },
    { itemKey: '2', format: 'files', operation: 'copy' },
  ] })
  expect(outcome.first.status).toBe('accepted')
  expect(outcome.first.items).toEqual([
    { itemKey: '0', status: 'accepted' },
    { itemKey: '1', status: 'failed', error: 'it no longer exists' },
    { itemKey: '2', status: 'accepted' },
  ])
  expect(outcome.same).toBe(true)
  const composer = hub.getByTestId('message-composer')
  await expect(composer.getByRole('button', { name: 'Remove Kyoto Trip Plan.md', exact: true })).toHaveCount(1)
  await expect(composer.getByRole('button', { name: 'Remove note.txt', exact: true })).toHaveCount(1)
  await expect(composer.getByTestId('zone-attach-failed')).toContainText('missing.txt')
})
