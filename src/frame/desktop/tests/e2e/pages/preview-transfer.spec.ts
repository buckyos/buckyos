import { CONTENT_REFS_FORMAT, contentRefsRepresentations } from 'buckyos/transfer'
import { expect, test, type Page } from '@playwright/test'
import { pickLocal } from '../fixtures/selectDialog'

/**
 * Preview as a source and a receiver of Zone content (剪贴板规范 §2.1, §2.2) on the mock runtime: the item being
 * previewed drags out from the toolbar's handle and is copied from More as a Zone reference; the landing page opens
 * a Zone file dropped on it like opening it from Files; archive members name their own address.
 */

async function openKyotoFromFiles(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-files').click()
  await page.getByTestId('window-files').getByRole('cell', { name: 'Documents', exact: true }).dblclick()
  await page.getByRole('cell', { name: /^Kyoto Trip Plan\.md/ }).first().dblclick()
  const previewWindow = page.getByTestId('window-preview')
  await expect(previewWindow.getByTestId('content-preview')).toHaveAttribute('data-status', 'ready', { timeout: 15_000 })
  return previewWindow
}

test('the previewed Zone file drags out from the toolbar handle and is copied from More as a reference', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  const previewWindow = await openKyotoFromFiles(page)
  const toolbar = previewWindow.getByTestId('content-preview-toolbar')
  await previewWindow.getByTestId('content-preview').hover()
  const handle = toolbar.getByTestId('content-preview-drag')
  await expect(handle).toBeVisible()
  await expect(handle).toHaveAttribute('draggable', 'true')
  await expect(handle).toHaveAccessibleName('Drag to another app')
  const written = await handle.evaluate((element) => {
    const transfer = new DataTransfer()
    element.dispatchEvent(new DragEvent('dragstart', { bubbles: true, cancelable: true, dataTransfer: transfer }))
    return { refs: transfer.getData('application/vnd.buckyos.content-refs+json'), text: transfer.getData('text/plain'), uris: transfer.getData('text/uri-list') }
  })
  expect(JSON.parse(written.refs).items).toEqual([{ origin: 'zone', kind: 'file', source: { kind: 'cyfs-path', path: 'cyfs:///home/Documents/Kyoto Trip Plan.md' }, hints: expect.objectContaining({ name: 'Kyoto Trip Plan.md' }) }])
  expect(written.text).toBe('Kyoto Trip Plan.md')
  expect(written.uris).toBe('cyfs:///home/Documents/Kyoto%20Trip%20Plan.md')

  // "Copy file" puts the same reference on the system clipboard: a readable name and the marked link.
  await toolbar.getByTestId('content-preview-more').click()
  await page.getByTestId('content-preview-menu-copy-file').click()
  const clipboard = await page.evaluate(async () => {
    const [item] = await navigator.clipboard.read()
    return { text: await (await item.getType('text/plain')).text(), html: await (await item.getType('text/html')).text() }
  })
  expect(clipboard.text).toBe('Kyoto Trip Plan.md')
  expect(clipboard.html).toContain('href="cyfs:///home/Documents/Kyoto%20Trip%20Plan.md"')
  expect(clipboard.html).toContain('data-buckyos-ref')
})

test('a file of this device has no Zone reference to drag or copy', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-preview').click()
  const previewWindow = page.getByTestId('window-preview')
  await previewWindow.getByTestId('preview-landing-choose').click()
  await pickLocal(page, { name: 'note.txt', mimeType: 'text/plain', buffer: Buffer.from('hello from this device') })
  await expect(previewWindow.getByTestId('content-preview-text')).toContainText('hello from this device', { timeout: 15_000 })
  await previewWindow.getByTestId('content-preview').hover()
  await expect(previewWindow.getByTestId('content-preview-drag')).toHaveCount(0)
  await previewWindow.getByTestId('content-preview-more').click()
  await expect(page.getByTestId('content-preview-menu-copy-file')).toHaveCount(0)
})

test('landing page: a Zone file dropped on it opens like opening it from Files; a folder is explained', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-preview').click()
  const previewWindow = page.getByTestId('window-preview')
  const landing = previewWindow.getByTestId('preview-landing')
  await expect(landing).toBeVisible()
  const drop = (data: Record<string, string>) => landing.evaluate((element, data) => {
    const transfer = new DataTransfer()
    for (const [type, value] of Object.entries(data)) transfer.setData(type, value)
    const over = new DragEvent('dragover', { bubbles: true, cancelable: true, dataTransfer: transfer })
    element.dispatchEvent(over)
    const dropped = new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer: transfer })
    element.dispatchEvent(dropped)
    return { accepted: over.defaultPrevented, claimed: dropped.defaultPrevented }
  }, data)

  expect(await drop(contentRefsRepresentations([{ origin: 'zone', kind: 'folder', source: { kind: 'cyfs-path', path: 'cyfs:///home/Pictures/sketches' }, hints: { name: 'sketches' } }]))).toEqual({ accepted: true, claimed: true })
  await expect(previewWindow.getByTestId('preview-landing-notice')).toHaveText('Folders cannot be previewed. Open them in Files.')

  expect(await drop(contentRefsRepresentations([{ origin: 'zone', kind: 'file', source: { kind: 'cyfs-path', path: 'cyfs:///home/Documents/Kyoto Trip Plan.md' }, hints: { name: 'Kyoto Trip Plan.md' } }]))).toEqual({ accepted: true, claimed: true })
  await expect(previewWindow.getByTestId('content-preview-text')).toContainText('Kyoto Trip Plan', { timeout: 15_000 })
  await expect(previewWindow.getByTestId('window-drag-preview')).toContainText('Kyoto Trip Plan.md')
})

test('archive members are dragged by their own address: the archive object and inner path, or the archive file path', async ({ page }) => {
  await page.goto('/?scenario=normal')
  const refs = await page.evaluate(async () => {
    const urls = '/src/app/filebrowser/data/urls.ts'
    const archive = '/src/components/preview/archive.ts'
    const source = '/src/components/preview/transferSource.ts'
    const { archiveObjectUrl, archiveRootUrl } = await import(urls)
    const { archiveSourceOf } = await import(archive)
    const { zoneContentOfPreviewSource } = await import(source)
    return [
      zoneContentOfPreviewSource(archiveSourceOf(archiveObjectUrl('cyfile:mock-zip', '/archive/docs/read me.pdf')), { name: 'read me.pdf' }),
      zoneContentOfPreviewSource(archiveSourceOf(archiveRootUrl('/home/Downloads/pack.zip', 'docs')), { name: 'docs' }, 'folder'),
    ]
  })
  expect(refs).toEqual([
    { origin: 'zone', kind: 'file', source: { kind: 'object-id', objectId: 'cyfile:mock-zip', innerPath: '/archive/docs/read me.pdf' }, hints: { name: 'read me.pdf', mime: 'application/pdf' } },
    { origin: 'zone', kind: 'folder', source: { kind: 'cyfs-path', path: 'cyfs:///home/Downloads/pack.zip/@/archive/docs' }, hints: { name: 'docs' } },
  ])
  const written = contentRefsRepresentations(refs)
  expect(Object.keys(written)).toEqual(expect.arrayContaining([CONTENT_REFS_FORMAT, 'text/plain', 'text/uri-list', 'text/html']))
  expect(written['text/plain']).toBe('read me.pdf\ndocs')
  expect(written['text/uri-list']).toBe('obj://cyfile:mock-zip/archive/docs/read%20me.pdf\r\ncyfs:///home/Downloads/pack.zip/%40/archive/docs')
})
