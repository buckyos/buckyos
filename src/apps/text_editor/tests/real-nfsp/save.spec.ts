import { test, expect, type APIRequestContext } from '@playwright/test'
import { readFile, writeFile, readdir } from 'node:fs/promises'
import { join } from 'node:path'
const NFSP = 'http://127.0.0.1:3261'
const bucketOf = (name: 'buffers' | 'recovery') => `user/text-editor.buckyos.bns.did/test-user/${name}`
const krpc = async (request: APIRequestContext, method: string, params: object) => (await (await request.post(`${NFSP}/kapi/nfs-server`, { data: { method, params, sys: [1] } })).json()).result
const keys = async (request: APIRequestContext, name: 'buffers' | 'recovery'): Promise<string[]> => (await krpc(request, 'bucket.list_objects', { bucket_id: bucketOf(name) })).items.map((i: { key: string }) => i.key)
const object = async (request: APIRequestContext, name: 'buffers' | 'recovery', key: string) => (await request.get(`${NFSP}/bucket/v1/${bucketOf(name)}/${key.split('/').map(encodeURIComponent).join('/')}`)).text()

test('real NFSP saves CRLF bytes, detects host writes, archives both conflict sides and persists buffers', async ({ page }) => {
  const home = join(process.env.TEXT_EDITOR_NFSP_ROOT!, 'home/test-user')
  const file = join(home, 'notes/demo.md')
  await page.goto('/?src=cyfs:///home/test-user/notes/demo.md')
  const editor = page.locator('.cm-content')
  await expect(editor).toContainText('Hello')
  await editor.click(); await page.keyboard.press('Control+a'); await page.keyboard.insertText('# Saved\nText\n'); await page.keyboard.press('Control+s')
  await expect(page.locator('.disk-state')).toHaveText('Saved')
  expect(await readFile(file, 'utf8')).toBe('# Saved\r\nText\r\n')
  await editor.click(); await page.keyboard.press('Control+a'); await page.keyboard.insertText('My changes')
  await writeFile(file, 'External version with different size')
  await page.keyboard.press('Control+s')
  await page.getByRole('button', { name: 'Reload (archive my changes)', exact: true }).click()
  await expect(editor).toContainText('External version')
  const archived = (await keys(page.request, 'recovery')).filter(key => key.endsWith('/demo.md'))
  expect(archived).toHaveLength(1)
  expect(await object(page.request, 'recovery', archived[0])).toBe('My changes')
  expect((await readdir(home)).filter(name => name.startsWith('.'))).toEqual([])
  await editor.click(); await page.keyboard.press('Control+a'); await page.keyboard.insertText('Second mine')
  await writeFile(file, 'Second external version, kept on disk')
  await page.keyboard.press('Control+s')
  await page.getByRole('button', { name: 'Overwrite and keep conflict copy', exact: true }).click()
  await expect(page.locator('.disk-state')).toHaveText('Saved')
  expect(await readFile(file, 'utf8')).toBe('Second mine')
  const copy = (await readdir(join(home, 'notes'))).find(name => name.includes('conflict copy'))!
  expect(await readFile(join(home, 'notes', copy), 'utf8')).toBe('Second external version, kept on disk')
  await editor.click(); await page.keyboard.press('Control+End'); await page.keyboard.insertText(' buffered')
  await expect(page.locator('.disk-state')).toHaveText('Unsaved (synced)')
  await expect.poll(async () => (await keys(page.request, 'buffers')).filter(key => key.endsWith('.buf')).length).toBe(1)
  const [buffer] = await keys(page.request, 'buffers')
  expect(await object(page.request, 'buffers', buffer)).toContain('Second mine buffered')
})

test('real NFSP: Save as creates nested folders, refuses a file in the way, and leaves created folders alone', async ({ page }) => {
  const home = join(process.env.TEXT_EDITOR_NFSP_ROOT!, 'home/test-user')
  await writeFile(join(home, 'notes/blocker'), 'a plain file')
  // Independent of the previous test, which leaves its buffer behind for this file.
  for (const key of await keys(page.request, 'buffers')) await krpc(page.request, 'bucket.delete', { bucket_id: bucketOf('buffers'), key })
  await page.goto('/?src=cyfs:///home/test-user/notes/demo.md')
  const original = (await readFile(join(home, 'notes/demo.md'), 'utf8')).replace(/\r\n/g, '\n')
  await expect(page.locator('.cm-content')).toContainText(original.split('\n')[0])
  const saveAs = async (folder: string, name: string) => {
    await page.getByRole('button', { name: 'Actions', exact: true }).first().click(); await page.getByRole('menuitem', { name: 'Save as…', exact: true }).click()
    await page.getByLabel('Location', { exact: true }).fill(folder)
    await page.getByLabel('File name', { exact: true }).fill(name)
    await page.getByRole('button', { name: 'Choose', exact: true }).click()
  }
  await saveAs('cyfs:///home/test-user/projects/2026/issues', 'note.md')
  await expect(page.locator('.disk-state')).toHaveText('Saved')
  expect((await readFile(join(home, 'projects/2026/issues/note.md'), 'utf8')).replace(/\r\n/g, '\n')).toBe(original)
  await saveAs('cyfs:///home/test-user/notes/blocker/sub', 'x.md')
  await expect(page.getByRole('status')).toContainText('a path part is a file')
  expect(await readFile(join(home, 'notes/blocker'), 'utf8')).toBe('a plain file')
  expect(await readdir(join(home, 'projects/2026/issues'))).toEqual(['note.md'])
})
