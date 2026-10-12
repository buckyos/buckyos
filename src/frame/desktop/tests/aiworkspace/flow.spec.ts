/* 流式页交互改进 (doc/workspace/流式页交互改进.md): the page as a document — handles, insertion where the user is,
 * keyboard; rich text without an editing step (shortcuts, `/`, format bubble, Markdown paste); the Markdown source
 * ("notepad") mode and its block-level sync; the table as a database grid. Real backend, sample workspace. */

import type { Page } from '@playwright/test'
import { expect, expectAllCommitted, openWorkspace, setCanvasMode, test, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'

// The backend's JSON results are inspected ad hoc.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

const notes = (page: Page) => page.getByTestId('aiws-richtext-notes')
const frame = (page: Page, cellId: string) => page.getByTestId(`aiws-cell-frame-${cellId}`)

async function order(api: Api, workspaceId: string, parent = 'surface-main'): Promise<string[]> {
  const children = (await api.rpc(ALICE, 'doc.list_children', { workspace_id: workspaceId, entity_id: parent })).children as { entity_id: string; type_id: string; view_type?: string; kind?: string }[]
  return children.filter((child) => child.type_id === 'buckyos.cell' || child.kind === 'group').map((child) => child.entity_id)
}

/** The editor document of the notes (test hook), top-level blocks only. */
async function editorBlocks(page: Page): Promise<Json[]> {
  return page.evaluate(() => (window as unknown as { __aiwsTestHooks: { editors: Record<string, () => { content: unknown[] }> } }).__aiwsTestHooks.editors.notes().content)
}

/** Put the caret on a new line after the last paragraph of the notes. */
async function newLineAtEnd(page: Page) {
  await notes(page).locator('[data-block-id="n-end"]').click()
  await page.keyboard.press('End')
  await page.keyboard.press('Enter')
}

const text = (node: Json): string => (node?.text ?? '') + ((node?.content ?? []) as Json[]).map(text).join('')

test('FP01 the page is a document: no frames, handles on hover, title renamed in place, view mode without handles', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `fp01 ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  const notesFrame = frame(page, 'cell-notes')
  await expect(notesFrame).toBeVisible()
  // no header row, no type label, no "编辑中": the text is the Block
  await expect(page.locator('[data-testid="aiws-flow"] .aiws-cell-head')).toHaveCount(0)
  await expect(page.getByTestId('aiws-flow')).not.toContainText('编辑中')
  await expect(notesFrame.locator('.aiws-inline-tools')).toHaveCount(0)
  // the handles show for the hovered Block only
  await notesFrame.hover()
  await expect(page.getByTestId('aiws-flow-handle-cell-notes')).toBeVisible()
  await expect(notesFrame.locator('.aiws-flow-gutter')).toHaveCSS('opacity', '1')
  await expect(frame(page, 'cell-info').locator('.aiws-flow-gutter')).toHaveCSS('opacity', '0')
  // a table carries its title line; clicking it renames the Block
  await expect(page.getByTestId('aiws-cell-title-cell-all-tasks')).toHaveText('全部任务')
  await page.getByTestId('aiws-cell-title-cell-all-tasks').click()
  await page.getByTestId('aiws-cell-title-input-cell-all-tasks').fill('任务总表')
  await page.getByTestId('aiws-cell-title-input-cell-all-tasks').press('Enter')
  await expect(page.getByTestId('aiws-cell-title-cell-all-tasks')).toHaveText('任务总表')
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, 'cell-all-tasks')).content.payload.title).toBe('任务总表')
  // the page title is renamed where it is shown
  await page.getByTestId('aiws-page-title').click()
  await page.getByTestId('aiws-page-title-input').fill('项目周报')
  await page.getByTestId('aiws-page-title-input').press('Enter')
  await expect(page.getByTestId('aiws-page-title')).toHaveText('项目周报')
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, 'surface-main')).content.payload.title).toBe('项目周报')
  await expectAllCommitted(page)
  // view mode reads: no ＋, no "continue writing", no input mode switch, the title is not editable
  await setCanvasMode(page, 'view')
  await notesFrame.hover()
  await expect(page.locator('[data-testid^="aiws-flow-plus-"]')).toHaveCount(0)
  await expect(page.getByTestId('aiws-flow-continue')).toHaveCount(0)
  await expect(page.getByTestId('aiws-md-toggle')).toHaveCount(0)
  await page.getByTestId('aiws-page-title').click()
  await expect(page.getByTestId('aiws-page-title-input')).toHaveCount(0)
})

test('FP02 Markdown shortcuts while typing, Backspace undoes one, Markdown pasted as text becomes formatted', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `fp02 ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  await newLineAtEnd(page)
  await page.keyboard.type('## 小结')
  await page.keyboard.press('Enter')
  await page.keyboard.type('- 第一条')
  await page.keyboard.press('Enter')
  await page.keyboard.type('第二条')
  // an empty item leaves the list
  await page.keyboard.press('Enter')
  await page.keyboard.press('Enter')
  await page.keyboard.type('这是**重点**和`代码`。')
  await page.keyboard.press('Enter')
  // "> " makes a quote; Backspace right after gives the typed characters back
  await page.keyboard.type('> ')
  await page.keyboard.press('Backspace')
  await page.keyboard.type('不是引用')
  await expectAllCommitted(page)
  const blocks = await editorBlocks(page)
  const heading = blocks.find((node) => node.type === 'heading' && text(node) === '小结')
  expect(heading?.attrs.level).toBe(2)
  const list = blocks.find((node) => node.type === 'bullet_list' && text(node).includes('第一条'))
  expect(list?.content.map(text)).toEqual(['第一条', '第二条'])
  const marked = blocks.find((node) => text(node).startsWith('这是'))
  expect(JSON.stringify(marked)).toContain('{"type":"strong"}')
  expect(JSON.stringify(marked)).toContain('{"type":"code"}')
  expect(text(marked)).toBe('这是重点和代码。')
  expect(blocks.some((node) => node.type === 'paragraph' && text(node) === '> 不是引用')).toBe(true)
  // the backend has the same document
  const ast = await api.ast(ALICE, ws.workspace_id, 'notes')
  expect(JSON.stringify(ast)).toContain('小结')

  // Markdown pasted as plain text
  await page.keyboard.press('Enter')
  await page.evaluate(() => {
    const data = new DataTransfer()
    data.setData('text/plain', '### 粘贴的标题\n\n1. 甲\n2. 乙\n\n[链接](https://example.com)')
    document.querySelector('[data-testid="aiws-richtext-notes"]')?.dispatchEvent(new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true }))
  })
  await expectAllCommitted(page)
  const after = await editorBlocks(page)
  expect(after.find((node) => node.type === 'heading' && text(node) === '粘贴的标题')?.attrs.level).toBe(3)
  expect(after.find((node) => node.type === 'ordered_list')?.content.map(text)).toEqual(['甲', '乙'])
  expect(JSON.stringify(after)).toContain('https://example.com')
})

test('FP03 the / menu: filters, turns the line into a heading, inserts a table right after the text', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `fp03 ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  await newLineAtEnd(page)
  await page.keyboard.type('/')
  const menu = page.getByTestId('aiws-slash-menu')
  await expect(menu).toBeVisible()
  await expect(menu.getByTestId('aiws-slash-h1')).toBeVisible()
  await expect(menu.getByTestId('aiws-slash-block-table')).toBeVisible()
  await page.keyboard.type('h3')
  await expect(menu.getByTestId('aiws-slash-h3')).toHaveAttribute('aria-selected', 'true')
  await expect(menu.getByTestId('aiws-slash-bullet')).toHaveCount(0)
  await page.keyboard.press('Enter')
  await expect(menu).toHaveCount(0)
  await page.keyboard.type('三级标题')
  await expectAllCommitted(page)
  const blocks = await editorBlocks(page)
  expect(blocks.find((node) => text(node) === '三级标题')?.attrs.level).toBe(3)
  expect(JSON.stringify(blocks)).not.toContain('/h3')
  // Esc closes the menu and keeps the slash as text
  await page.keyboard.press('Enter')
  await page.keyboard.type('/')
  await expect(menu).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(menu).toHaveCount(0)
  await page.keyboard.press('Backspace')
  // a table from the menu goes right after this text Block
  await page.keyboard.type('/表格')
  await page.getByTestId('aiws-slash-block-table').click()
  await expect.poll(async () => (await order(api, ws.workspace_id)).length).toBe(6)
  const now = await order(api, ws.workspace_id)
  expect(now[0]).toBe('cell-notes')
  expect(now.slice(2)).toEqual(['cell-all-tasks', 'cell-open-tasks', 'cell-info', 'cell-diagram'])
  await expect(frame(page, now[1])).toBeVisible()
  await expect(page.getByTestId(`aiws-cell-title-${now[1]}`)).toHaveText('无标题')
  await expectAllCommitted(page)
})

test('FP04 Markdown source ("notepad") mode: typed text reaches the rich text block by block, ids survive, the mode is remembered', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `fp04 ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  await frame(page, 'cell-notes').hover()
  await page.getByTestId('aiws-md-toggle').first().click()
  const source = page.getByTestId('aiws-md-source')
  await expect(source).toBeVisible()
  await expect(source).toBeFocused()
  const initial = await source.inputValue()
  expect(initial.startsWith('# 本周项目进展\n\n')).toBe(true)
  expect(initial).toContain('[任务 42 详情](entity:task-42-details)')
  expect(initial).toContain('](embed:cell-open-tasks)')
  // typed at the end: new blocks after the last one
  await source.press('Control+End')
  await page.keyboard.type('\n\n> 引用一段\n\n1. 一\n2. 二')
  await expect.poll(async () => JSON.stringify(await api.ast(ALICE, ws.workspace_id, 'notes'))).toContain('引用一段')
  // a change inside one block replaces that block only, keeping its id (annotations stay anchored)
  const value = await source.inputValue()
  await source.fill(value.replace('# 本周项目进展', '# 本周进展').replace('- 对象与稳定引用', '- 对象与**稳定**引用'))
  await expect.poll(async () => JSON.stringify(await api.ast(ALICE, ws.workspace_id, 'notes'))).toContain('本周进展')
  await expectAllCommitted(page)
  const ast = await api.ast(ALICE, ws.workspace_id, 'notes')
  const top = ast.content as Json[]
  expect(top.map((node) => node.attrs?.block_id).slice(0, 5)).toEqual(['n-title', 'n-intro', 'n-list', 'n-embed', 'n-end'])
  expect(text(top[0])).toBe('本周进展')
  expect(top[3].type).toBe('object_embed')
  expect(top[3].attrs.ref.entity_id).toBe('cell-open-tasks')
  expect(JSON.stringify(top[1])).toContain('task-42-details')
  expect(JSON.stringify(top[2])).toContain('{"type":"strong"}')
  expect(top.some((node) => node.type === 'blockquote')).toBe(true)
  expect(top.find((node) => node.type === 'ordered_list')?.content.map(text)).toEqual(['一', '二'])
  // the text the user typed is never rewritten under the caret
  expect(await source.inputValue()).toContain('- 对象与**稳定**引用')
  // remembered on this device: opened again, the text is in source mode
  await openWorkspace(page, ALICE, ws.workspace_id)
  await expect(page.getByTestId('aiws-md-source')).toBeVisible({ timeout: 30_000 })
  // back to the formatted view (Ctrl+/): the caret stays in the text
  await page.getByTestId('aiws-md-source').press('Control+/')
  await expect(page.getByTestId('aiws-md-source')).toHaveCount(0)
  await expect(notes(page).locator('h1')).toHaveText('本周进展')
  await expect(notes(page).locator('blockquote')).toContainText('引用一段')
})

test('FP05 source mode and a colleague: changes from elsewhere come in, my typing is kept', async ({ browser, api }) => {
  const ws = await api.sample(ALICE, `fp05 ${Date.now()}`)
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read', 'update', 'append', 'comment'] })
  const aliceContext = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const bobContext = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const alice = await aliceContext.newPage()
  const bob = await bobContext.newPage()
  await openWorkspace(alice, ALICE, ws.workspace_id)
  await openWorkspace(bob, BOB, ws.workspace_id)
  await frame(alice, 'cell-notes').hover()
  await alice.getByTestId('aiws-md-toggle').first().click()
  const source = alice.getByTestId('aiws-md-source')
  await source.press('Control+End')
  await alice.keyboard.type('\n\n甲在源码里写的一段')
  await expect.poll(async () => JSON.stringify(await api.ast(ALICE, ws.workspace_id, 'notes'))).toContain('甲在源码里写的一段')
  // bob edits the title in the formatted view
  await notes(bob).locator('[data-block-id="n-title"]').click()
  await bob.keyboard.press('End')
  await bob.keyboard.type('（乙）')
  await expect(source).toHaveValue(/^# 本周项目进展（乙）\n/, { timeout: 20_000 })
  await expect(source).toHaveValue(/甲在源码里写的一段/)
  await aliceContext.close()
  await bobContext.close()
})

test('FP06 keyboard and handles: Esc selects the Block, arrows walk, Ctrl+Shift+arrows and dragging move, menu deletes', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `fp06 ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  await notes(page).locator('[data-block-id="n-intro"]').click()
  await page.keyboard.press('Escape')
  await expect(frame(page, 'cell-notes')).toHaveClass(/is-selected/)
  await expect(notes(page)).not.toBeFocused()
  await page.keyboard.press('ArrowDown')
  await expect(frame(page, 'cell-all-tasks')).toHaveClass(/is-selected/)
  await expect(frame(page, 'cell-notes')).not.toHaveClass(/is-selected/)
  await page.keyboard.press('Control+Shift+ArrowUp')
  await expect.poll(() => order(api, ws.workspace_id)).toEqual(['cell-all-tasks', 'cell-notes', 'cell-open-tasks', 'cell-info', 'cell-diagram'])
  await page.keyboard.press('Escape')
  await expect(frame(page, 'cell-all-tasks')).not.toHaveClass(/is-selected/)
  // drag the record before the first Block
  await frame(page, 'cell-info').hover()
  const handle = await page.getByTestId('aiws-flow-handle-cell-info').boundingBox()
  const target = await frame(page, 'cell-all-tasks').boundingBox()
  if (!handle || !target) throw new Error('no boxes')
  await page.mouse.move(handle.x + handle.width / 2, handle.y + handle.height / 2)
  await page.mouse.down()
  for (let i = 1; i <= 10; i++) await page.mouse.move(handle.x + handle.width / 2 + i, handle.y + (target.y + 6 - handle.y) * i / 10)
  await expect(page.getByTestId('aiws-flow-drop')).toBeVisible()
  await page.mouse.up()
  await expect.poll(() => order(api, ws.workspace_id)).toEqual(['cell-info', 'cell-all-tasks', 'cell-notes', 'cell-open-tasks', 'cell-diagram'])
  await expect.poll(() => page.locator('[data-testid="aiws-flow"] > [data-cell-id]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute('data-cell-id'))))
    .toEqual(['cell-info', 'cell-all-tasks', 'cell-notes', 'cell-open-tasks', 'cell-diagram'])
  // a click on ⋮⋮ selects and opens the menu; deleting the picture Block from it
  await frame(page, 'cell-diagram').hover()
  await page.getByTestId('aiws-flow-handle-cell-diagram').click()
  await expect(frame(page, 'cell-diagram')).toHaveClass(/is-selected/)
  await page.getByTestId('aiws-delete-block-cell-diagram').click()
  await expect(frame(page, 'cell-diagram')).toHaveCount(0)
  await expectAllCommitted(page)
})

test('FP07 insertion where the user is: ＋ after a Block, and a click below the page continues writing', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `fp07 ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  await frame(page, 'cell-notes').hover()
  await page.getByTestId('aiws-flow-plus-cell-notes').click()
  await page.getByTestId('aiws-flow-add-richtext').click()
  // a new text right after the notes, with the caret in it
  await expect.poll(async () => (await order(api, ws.workspace_id)).length).toBe(6)
  const textCell = (await order(api, ws.workspace_id))[1]
  await expect(frame(page, textCell).locator('.ProseMirror')).toBeFocused()
  await page.keyboard.type('插在说明后面')
  await expectAllCommitted(page)
  const textSource = (await api.read(ALICE, ws.workspace_id, textCell)).content.payload.source_ref.entity_id
  expect(text(await api.ast(ALICE, ws.workspace_id, textSource))).toContain('插在说明后面')
  // below the last Block (a picture): a click starts a new text there
  await page.getByTestId('aiws-flow-continue').click()
  await expect.poll(async () => (await order(api, ws.workspace_id)).length).toBe(7)
  const lastCell = (await order(api, ws.workspace_id))[6]
  await expect(frame(page, lastCell).locator('.ProseMirror')).toBeFocused()
  await page.keyboard.type('页尾第一段')
  await expectAllCommitted(page)
  const lastSource = (await api.read(ALICE, ws.workspace_id, lastCell)).content.payload.source_ref.entity_id
  // the last Block is text now: the next click continues it on a new line instead of adding a Block
  await page.getByTestId('aiws-flow-continue').click()
  await expect(frame(page, lastCell).locator('.ProseMirror')).toBeFocused()
  await page.keyboard.type('页尾第二段')
  await expectAllCommitted(page)
  expect((await order(api, ws.workspace_id)).length).toBe(7)
  const ast = await api.ast(ALICE, ws.workspace_id, lastSource)
  expect((ast.content as Json[]).map(text)).toEqual(['页尾第一段', '页尾第二段'])
})

test('FP08 the table as a database grid: new rows by typing, a field from the header, tick boxes, keyboard between cells', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `fp08 ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  const all = frame(page, 'cell-all-tasks')
  const rows = async () => (await api.rpc(ALICE, 'doc.query', { workspace_id: ws.workspace_id, source_id: 'tasks' })).rows as { record_id: string; values: Record<string, unknown> }[]
  // field types show as icons in the header; options as pills
  await expect(all.locator('.aiws-th').first().locator('svg')).toBeVisible()
  await expect(all.getByTestId('aiws-cell-task-40-status').locator('.aiws-pill')).toHaveText('完成')
  // "+ 新建": type, Enter, type the next one
  await all.getByTestId('aiws-add-record-cell-all-tasks').click()
  await page.getByTestId('aiws-new-record-input').fill('第一条新任务')
  await page.getByTestId('aiws-new-record-input').press('Enter')
  await expect(page.getByTestId('aiws-new-record-input')).toHaveValue('')
  await page.getByTestId('aiws-new-record-input').fill('第二条新任务')
  await page.getByTestId('aiws-new-record-input').press('Enter')
  await expect.poll(async () => (await rows()).length).toBe(7)
  await page.getByTestId('aiws-new-record-input').press('Escape')
  await expect(page.getByTestId('aiws-new-record-input')).toHaveCount(0)
  await expect(all.getByTestId('aiws-row').filter({ hasText: '第二条新任务' })).toBeVisible()
  // a field from the header's "+": name and type only
  await all.getByTestId('aiws-add-field-open-cell-all-tasks').click()
  await page.getByTestId('aiws-quick-field-name').fill('已复核')
  await page.getByTestId('aiws-quick-field-type-boolean').click()
  await page.getByTestId('aiws-quick-field-add').click()
  await expect(all.locator('.aiws-th').filter({ hasText: '已复核' })).toBeVisible()
  const fieldId = ((await api.read(ALICE, ws.workspace_id, 'tasks')).content.fields as { field_id: string; name: string }[]).find((f) => f.name === '已复核')?.field_id as string
  // a tick box writes with one click
  await page.getByLabel('已复核 task-41', { exact: true }).check()
  await expect.poll(async () => (await rows()).find((row) => row.record_id === 'task-41')?.values[fieldId]).toBe(true)
  // arrows move between cells; a typed character starts editing with it; Tab commits and moves on
  await all.getByTestId('aiws-cell-task-40-title').locator('button.aiws-cell-value').focus()
  await page.keyboard.press('ArrowDown')
  await expect(all.getByTestId('aiws-cell-task-41-title').locator('button.aiws-cell-value')).toBeFocused()
  for (let i = 0; i < 4; i++) await page.keyboard.press('ArrowRight')
  await expect(all.getByTestId('aiws-cell-task-41-owner').locator('button.aiws-cell-value')).toBeFocused()
  // (an ASCII key: Playwright sends no keydown for characters outside a US layout)
  await page.keyboard.press('Z')
  await expect(page.getByLabel('负责人 task-41', { exact: true })).toHaveValue('Z')
  await page.keyboard.press('Tab')
  await expect.poll(async () => (await rows()).find((row) => row.record_id === 'task-41')?.values.owner).toBe('Z')
  // a row is deleted from its hover button
  await all.getByTestId('aiws-row').filter({ hasText: '第一条新任务' }).hover()
  const added = (await rows()).find((row) => row.values.title === '第一条新任务')?.record_id as string
  await page.getByTestId(`aiws-delete-record-${added}`).click()
  await expect.poll(async () => (await rows()).length).toBe(6)
  await expectAllCommitted(page)
})
