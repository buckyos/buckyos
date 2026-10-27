import { crc32, deflateSync } from 'node:zlib'
import { expect, test, type Page } from '@playwright/test'

declare global {
  interface Window { __messageHubMock: import('../../../src/app/messagehub/mock/store').MessageHubMockStore }
}

const SELF = 'did:buckyos:user:self'
const CODER = 'did:buckyos:agent:codeassistant'
const ALICE = 'did:buckyos:person:alice'
const OWN = { viewerDid: SELF, ownerDid: SELF, mode: 'self' as const }

function png(width: number, height: number, rgb: [number, number, number]): Buffer {
  const chunk = (type: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(type, 'ascii'), data])
    const length = Buffer.alloc(4)
    length.writeUInt32BE(data.length)
    const crc = Buffer.alloc(4)
    crc.writeUInt32BE(crc32(body))
    return Buffer.concat([length, body, crc])
  }
  const header = Buffer.alloc(13)
  header.writeUInt32BE(width, 0)
  header.writeUInt32BE(height, 4)
  header[8] = 8
  header[9] = 2
  const row = Buffer.concat([Buffer.from([0]), Buffer.concat(Array.from({ length: width }, () => Buffer.from(rgb)))])
  const pixels = Buffer.concat(Array.from({ length: height }, () => row))
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', header), chunk('IDAT', deflateSync(pixels)), chunk('IEND', Buffer.alloc(0))])
}

async function openSession(page: Page, title: string) {
  await page.goto(`/messagehub?entityId=${encodeURIComponent(CODER)}`)
  await expect(page.getByRole('button', { name: 'Sessions', exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'New Session', exact: true }).last().click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('Title (optional)', { exact: true }).fill(title)
  await dialog.getByRole('combobox', { name: 'Connection', exact: true }).selectOption('native')
  await dialog.getByRole('button', { name: 'Create', exact: true }).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-raw-count', '0')
  return page.evaluate(({ context, entity }) => window.__messageHubMock.sessions(context, entity)[0].id, { context: OWN, entity: CODER })
}

for (const scheme of ['light', 'dark'] as const) {
  test(`a picture shows at once with its upload progress, and later sends wait in order (${scheme})`, async ({ page }) => {
    const errors: string[] = []
    page.on('pageerror', error => errors.push(error.message))
    await page.addInitScript(value => window.localStorage.setItem('buckyos.prototype.theme.v1', value), scheme)
    await page.setViewportSize({ width: 390, height: 844 })
    const id = await openSession(page, `Upload ${scheme}`)
    await page.evaluate(() => window.__messageHubMock.configure({ uploadMs: 3000 }))
    const history = page.getByTestId('conversation-history')
    const textarea = page.locator('textarea')

    await page.locator('input[type="file"]').first().setInputFiles({ name: 'beach.png', mimeType: 'image/png', buffer: png(320, 200, [40, 140, 220]) })
    await textarea.fill('Look at this')
    await textarea.press('Enter')
    const progress = history.getByTestId('upload-progress')
    await expect(progress).toBeVisible({ timeout: 1_000 })
    await expect(history.getByRole('img', { name: 'beach.png', exact: true })).toBeVisible({ timeout: 1_000 })
    await expect(history.getByTestId('delivery-status').first()).toHaveAttribute('data-status', 'sending')

    await textarea.fill('Second message')
    await textarea.press('Enter')
    await expect(history.getByText('Second message', { exact: true })).toBeVisible({ timeout: 1_000 })
    await expect(textarea).toHaveValue('')
    await expect(history).toHaveAttribute('data-raw-count', '2')
    const order = await history.evaluate(element => {
      const text = element.textContent ?? ''
      return [text.indexOf('Look at this'), text.indexOf('Second message')]
    })
    expect(order[0]).toBeGreaterThanOrEqual(0)
    expect(order[0]).toBeLessThan(order[1])
    await expect(progress).toHaveAttribute('data-phase', 'uploading')
    await expect.poll(async () => Number(await progress.getAttribute('data-percent'))).toBeGreaterThan(0)
    await page.screenshot({ path: `test-results/messagehub-upload-progress-${scheme}.png` })

    await expect(progress).toHaveCount(0, { timeout: 10_000 })
    await expect(history.getByRole('img', { name: 'beach.png', exact: true })).toBeVisible()
    await expect(history).toHaveAttribute('data-raw-count', '2')
    const messages = await page.evaluate(({ context, id }) => window.__messageHubMock.reader(context, id).readRange(0, 5), { context: OWN, id })
    expect(messages).toHaveLength(2)
    expect(messages[0].content.content).toBe('Look at this')
    expect(messages[0].content.refs?.[0]?.target).toMatchObject({ type: 'data_obj', obj_id: expect.stringMatching(/^cyfile:mock-upload-/) })
    expect(messages[0].content.refs?.[0]?.label).toBe('beach.png')
    expect(messages[1].content.content).toBe('Second message')
    expect(messages[0].created_at_ms).toBeLessThan(messages[1].created_at_ms)
    await page.screenshot({ path: `test-results/messagehub-upload-done-${scheme}.png` })

    await page.reload()
    await expect(page.getByTestId('conversation-history').getByRole('img', { name: 'beach.png', exact: true })).toBeVisible({ timeout: 15_000 })
    expect(errors).toEqual([])
  })
}

test('a failed upload takes its bubble back and returns the draft to the composer', async ({ page }) => {
  await openSession(page, 'Upload failure')
  await page.evaluate(() => window.__messageHubMock.configure({ uploadMs: 800, failNext: 'send_failure' }))
  const history = page.getByTestId('conversation-history')
  await page.locator('input[type="file"]').first().setInputFiles({ name: 'beach.png', mimeType: 'image/png', buffer: png(32, 20, [200, 80, 40]) })
  await page.locator('textarea').fill('Will fail')
  await page.locator('textarea').press('Enter')
  await expect(history.getByTestId('upload-progress')).toBeVisible({ timeout: 1_000 })
  await page.locator('textarea').fill('Queued behind')
  await page.locator('textarea').press('Enter')
  await expect(history.getByText('Queued behind', { exact: true })).toBeVisible({ timeout: 1_000 })
  await expect(page.getByTestId('message-composer').getByRole('alert')).toBeVisible({ timeout: 10_000 })
  await expect(history).toHaveAttribute('data-raw-count', '0')
  await expect(page.locator('textarea')).toHaveValue('Will fail\nQueued behind')
  await expect(page.getByTitle('beach.png', { exact: true })).toBeVisible()
})

test('an iPhone HEIC photo the browser can decode goes out as a JPEG; one it cannot decode goes out unchanged', async ({ page }) => {
  const id = await openSession(page, 'HEIC photos')
  const input = page.locator('input[type="file"]').first()
  await input.setInputFiles({ name: 'IMG_0001.HEIC', mimeType: 'image/heic', buffer: png(64, 48, [250, 200, 20]) })
  await input.setInputFiles({ name: 'IMG_0002.HEIC', mimeType: 'image/heic', buffer: Buffer.concat([Buffer.from([0, 0, 0, 24]), Buffer.from('ftypheic', 'ascii'), Buffer.alloc(64)]) })
  await page.locator('textarea').press('Enter')
  await expect.poll(() => page.evaluate(async ({ context, id }) => (await window.__messageHubMock.reader(context, id).readRange(0, 1)).map(message => message.ui_local_id ?? 'stored'), { context: OWN, id }), { timeout: 10_000 }).toEqual(['stored'])
  const sent = await page.evaluate(async ({ context, id }) => {
    const [message] = await window.__messageHubMock.reader(context, id).readRange(0, 1)
    const modulePath = '/src/app/messagehub/conversation/history/objectAccess.ts'
    const { getObjectAccess } = await import(modulePath)
    return Promise.all((message.content.refs ?? []).map(async ref => {
      const info = await getObjectAccess()!.describe(ref.target.type === 'data_obj' ? ref.target.obj_id : '')
      return { label: ref.label, mimeType: info.mimeType }
    }))
  }, { context: OWN, id })
  expect(sent).toEqual([{ label: 'IMG_0001.jpg', mimeType: 'image/jpeg' }, { label: 'IMG_0002.HEIC', mimeType: 'image/heic' }])
  await expect(page.getByTestId('conversation-history').getByRole('img', { name: 'IMG_0001.jpg', exact: true })).toBeVisible()
})

test('photos with the same file name are all kept in the draft', async ({ page }) => {
  await openSession(page, 'Camera photos')
  const input = page.locator('input[type="file"]').first()
  await input.setInputFiles({ name: 'image.jpg', mimeType: 'image/png', buffer: png(30, 20, [10, 200, 10]) })
  await input.setInputFiles({ name: 'image.jpg', mimeType: 'image/png', buffer: png(40, 20, [200, 10, 10]) })
  await expect(page.getByTestId('message-composer').getByRole('button', { name: 'Remove image.jpg', exact: true })).toHaveCount(2)
})

test('a file attachment downloads from its own button', async ({ page }) => {
  await page.route('https://upload.wikimedia.org/**', route => route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"/>' }))
  await page.goto(`/messagehub?entityId=${encodeURIComponent(ALICE)}`)
  const card = page.getByTestId('conversation-history').getByTestId('attachment-file').filter({ hasText: 'design-brief.pdf' })
  await expect(card).toBeVisible({ timeout: 15_000 })
  await card.scrollIntoViewIfNeeded()
  await page.screenshot({ path: 'test-results/messagehub-attachment-download.png' })
  const download = page.waitForEvent('download')
  await card.getByRole('button', { name: 'Download design-brief.pdf', exact: true }).click()
  expect((await download).suggestedFilename()).toBe('design-brief.pdf')
  await expect(page.getByTestId('messagehub-media-viewer')).toHaveCount(0)
})

test('an attachment is saved to the viewer\'s collection through its message', async ({ page }) => {
  await page.route('https://upload.wikimedia.org/**', route => route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"/>' }))
  await page.goto(`/messagehub?entityId=${encodeURIComponent(ALICE)}`)
  const card = page.getByTestId('conversation-history').getByTestId('attachment-file').filter({ hasText: 'design-brief.pdf' })
  await expect(card).toBeVisible({ timeout: 15_000 })
  await card.scrollIntoViewIfNeeded()
  await card.getByRole('button', { name: 'Save design-brief.pdf to my collection', exact: true }).click()
  const saved = card.getByRole('button', { name: 'Saved to my collection', exact: true })
  await expect(saved).toHaveAttribute('data-state', 'saved')
  await page.screenshot({ path: 'test-results/messagehub-attachment-collect.png' })
  const collected = await page.evaluate(async () => {
    const modulePath = '/src/app/messagehub/mock/objects.ts'
    const { mockCollected } = await import(modulePath)
    return [...mockCollected.entries()]
  })
  expect(collected).toHaveLength(1)
  expect(collected[0][0]).toMatch(/\/design-brief\.pdf$/)
  expect(collected[0][1]).toBe('cyfile:mock-doc-brief')
})

test('directory enumeration retains empty nodes and reports file or reader failures', async ({ page }) => {
  await page.goto('/messagehub')
  const result = await page.evaluate(async () => {
    const modulePath = '/src/app/messagehub/conversation/input/attachmentDraft.ts'
    const { extractTransferFiles } = await import(modulePath)
    const transfer = (entry: object) => ({ items: [{ kind: 'file', webkitGetAsEntry: () => entry }], files: [] })
    const directory = (path: string, entries: object[]) => ({ isDirectory: true, isFile: false, fullPath: path, createReader() { let sent = false; return { readEntries(done: (items: object[]) => void) { done(sent ? [] : entries); sent = true } } } })
    const selected = await extractTransferFiles(transfer(directory('/旅行', [directory('/旅行/空目录', []), { isFile: true, fullPath: '/旅行/photo.heic', file(done: (file: File) => void) { done(new File(['original'], 'photo.heic', { type: 'image/heic' })) } }])) as unknown as DataTransfer)
    const failures: string[] = []
    for (const entry of [
      { isFile: true, fullPath: '/旅行/unreadable.txt', file(_done: unknown, fail: (error: DOMException) => void) { fail(new DOMException('permission denied')) } },
      { isDirectory: true, fullPath: '/旅行', createReader: () => ({ readEntries(_done: unknown, fail: (error: DOMException) => void) { fail(new DOMException('directory unavailable')) } }) },
    ]) {
      try { await extractTransferFiles(transfer(entry) as unknown as DataTransfer) } catch (error) { failures.push(String(error)) }
    }
    return { count: selected.length, root: selected[0].directory?.name, empty: selected[0].directory?.children[0], failures, bytes: await selected[0].directory?.children[1].file.text() }
  })
  expect(result).toMatchObject({ count: 1, root: '旅行', empty: { kind: 'dir', name: '空目录', children: [] }, bytes: 'original' })
  expect(result.failures[0]).toContain('unreadable.txt: permission denied')
  expect(result.failures[1]).toContain('旅行: directory unavailable')
})

test('a directory is one attachment with empty children and original file bytes after reload', async ({ page }) => {
  const sessionId = await openSession(page, 'Directory attachment')
  const bytes = await page.evaluate(async ({ context, sessionId }) => {
    const directory = { kind: 'dir' as const, name: '旅行', children: [
      { kind: 'dir' as const, name: '空目录', children: [] },
      { kind: 'file' as const, file: new File(['original HEIC bytes'], 'photo.heic', { type: 'image/heic' }) },
    ] }
    await window.__messageHubMock.send(context, sessionId, { content: 'folder', attachments: [{ file: new File([], directory.name), directory }] }, undefined)
    const [message] = await window.__messageHubMock.reader(context, sessionId).readRange(0, 1)
    const target = message.content.refs?.[0].target
    if (target?.type !== 'data_obj') throw Error('missing directory reference')
    const modulePath = '/src/app/messagehub/conversation/history/objectAccess.ts'
    const { getObjectAccess } = await import(modulePath)
    return { count: message.content.refs?.length, id: target.obj_id, content: await (await fetch(await getObjectAccess()!.contentUrl(target.obj_id, undefined, 'photo.heic'))).text() }
  }, { context: OWN, sessionId })
  expect(bytes).toMatchObject({ count: 1, id: expect.stringMatching(/^cydir:/), content: 'original HEIC bytes' })
  await page.reload()
  const folder = page.getByTestId('conversation-history').getByTestId('attachment-directory').first()
  await folder.getByRole('button', { name: '旅行', exact: true }).click()
  await expect(folder.getByRole('button', { name: '空目录', exact: true })).toBeVisible()
  await expect(folder.getByRole('button', { name: 'photo.heic', exact: true })).toBeVisible()
})

test('a directory read failure names the failed path in the composer', async ({ page }) => {
  await openSession(page, 'Directory failure')
  await page.getByTestId('message-composer').evaluate(element => {
    const transfer = new DataTransfer()
    transfer.items.add(new File([''], 'unreadable.txt'))
    Object.defineProperty(transfer, 'items', { value: [{ kind: 'file', webkitGetAsEntry: () => ({ isFile: true, fullPath: '/旅行/unreadable.txt', file(_done: unknown, fail: (error: DOMException) => void) { fail(new DOMException('permission denied')) } }) }] })
    element.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer: transfer }))
  })
  await expect(page.getByTestId('message-composer').getByRole('alert')).toContainText('unreadable.txt: permission denied')
  await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-raw-count', '0')
})
