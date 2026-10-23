import { test, expect } from '@playwright/test'
import { readFile, writeFile, readdir, rm } from 'node:fs/promises'
import { join } from 'node:path'

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
  const data = join(home, '.local/share/text-editor.buckyos.bns.did')
  const entries = await readdir(join(data, 'recovery'))
  expect(await readFile(join(data, 'recovery', entries[0], 'demo.md'), 'utf8')).toBe('My changes')
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
  const buffers = await readdir(join(data, 'buffers'))
  expect(buffers.filter(name => name.endsWith('.buf'))).toHaveLength(1)
  expect(await readFile(join(data, 'buffers', buffers[0]), 'utf8')).toContain('Second mine buffered')
})

test('real NFSP: Save as creates nested folders, refuses a file in the way, and leaves created folders alone', async ({ page }) => {
  const home = join(process.env.TEXT_EDITOR_NFSP_ROOT!, 'home/test-user')
  await writeFile(join(home, 'notes/blocker'), 'a plain file')
  // Independent of the previous test, which leaves its buffer behind for this file.
  await rm(join(home, '.local/share/text-editor.buckyos.bns.did/buffers'), { recursive: true, force: true })
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
