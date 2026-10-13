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
