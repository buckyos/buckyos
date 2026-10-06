import type { Page } from '@playwright/test'
import { expect, expectAllCommitted, openWorkspace, test, type Api } from './fixtures'

/* Annotation anchors (design §3.7): select → annotate → shown next to the selection; the anchor
 * follows edits, is found again by its quote, and falls back by grade; an application's own range
 * kind is captured and located by the application and kept verbatim by the backend. */

const ALICE = 'tok-alice'

/** Select the contents of the first element matching `selector` inside block `blockId` of `notes`, as a user's drag would (focused editor). */
async function selectInBlock(page: Page, blockId: string, selector: string, text: string) {
  await page.evaluate(({ blockId, selector, text }) => {
    const editor = document.querySelector<HTMLElement>('[data-testid="aiws-richtext-notes"]')
    const block = editor?.querySelector(`[data-block-id="${blockId}"]`)
    const target = [...(block?.querySelectorAll(selector) ?? [])].find((element) => element.textContent === text)
    if (!editor || !target) throw new Error(`no ${selector} "${text}" in ${blockId}`)
    editor.focus()
    const range = document.createRange()
    range.selectNodeContents(target)
    const selection = window.getSelection()
    selection?.removeAllRanges()
    selection?.addRange(range)
  }, { blockId, selector, text })
}

async function annotation(api: Api, ws: { workspace_id: string }, body: string) {
  const list = await api.rpc(ALICE, 'doc.list_annotations', { workspace_id: ws.workspace_id, target_ids: ['notes'] })
  return (list.annotations as { content: { payload: { body: string; range?: unknown }; anchor: Record<string, unknown> } }[])
    .find((item) => item.content.payload.body === body)?.content
}

async function block(api: Api, ws: { workspace_id: string }, blockId: string) {
  return (await api.rpc(ALICE, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'notes' })).content.blocks[blockId] as { hash: string; struct_rev: number }
}

test('a text range: shown next to the selection, follows edits, found again by its quote, falls back by grade', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `anchor ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  const frame = page.getByTestId('aiws-cell-frame-cell-notes')
  const prose = page.getByTestId('aiws-richtext-notes')
  const item = page.getByTestId('aiws-annotation').filter({ hasText: '术语要统一' })
  const card = frame.getByTestId('aiws-anno-card').filter({ hasText: '术语要统一' })

  // select a few characters of the first paragraph and annotate them
  await expect(prose.locator('[data-block-id="n-intro"] strong', { hasText: '文档格式' })).toBeVisible()
  await selectInBlock(page, 'n-intro', 'strong', '文档格式')
  await frame.getByTestId('aiws-annotate').click()
  await expect(page.getByTestId('aiws-annotation-draft-target')).toContainText('「文档格式」')
  await page.getByLabel('批注内容').fill('术语要统一')
  await page.getByTestId('aiws-annotation-save').click()
  await expect(item).toContainText('锚点有效')

  // the selection is highlighted and the card sits next to it
  const highlight = prose.locator('.aiws-anno-range')
  await expect(highlight).toHaveText('文档格式')
  await expect(card).toBeVisible()
  const [mark, box] = [await highlight.boundingBox(), await card.boundingBox()]
  expect(Math.abs((mark?.y ?? 0) - (box?.y ?? 1000))).toBeLessThan(24)
  expect(box?.x ?? 0).toBeGreaterThan((mark?.x ?? 0) + (mark?.width ?? 0))
  expect(await annotation(api, ws, '术语要统一')).toMatchObject({
    payload: { range: { kind: 'richtext_text', start: { block_id: 'n-intro' }, end: { block_id: 'n-intro' } } },
    anchor: { state: 'resolved', level: 'range', range_status: 'exact', position: { block_id: 'n-intro', text: '文档格式' } },
  })
  // the card and the text select each other
  await card.click()
  await expect(highlight).toHaveClass(/is-active/)
  await expect(item).toHaveClass(/is-active/)

  // typing in front of it: the Loro cursors follow, locally and in the backend
  await prose.locator('[data-block-id="n-intro"]').click({ position: { x: 2, y: 4 } })
  await page.keyboard.press('Home')
  await page.keyboard.type('【补】')
  await expectAllCommitted(page)
  await expect(highlight).toHaveText('文档格式')
  await expect.poll(async () => (await annotation(api, ws, '术语要统一'))?.anchor).toMatchObject({ range_status: 'exact', position: { text: '文档格式' } })

  // a block operation rebuilds the paragraph elsewhere: the cursors are gone, the quote finds the text
  const intro = await block(api, ws, 'n-intro')
  expect((await api.commit(ALICE, ws, [{ op: 'richtext.move_block', entity_id: 'notes', block_id: 'n-intro', expect: { struct_rev: intro.struct_rev }, position: { after: 'n-end' } }])).status).toBe('accepted')
  await expect(item.getByTestId('aiws-anchor-state')).toHaveText('已按原文重新定位')
  await expect(highlight).toHaveText('文档格式')
  await expect(card.getByTestId('aiws-anno-card-hint')).toHaveText('已按原文重新定位')

  // the text is rewritten: shown on its paragraph
  const moved = await block(api, ws, 'n-intro')
  expect((await api.commit(ALICE, ws, [{ op: 'richtext.replace_block', entity_id: 'notes', block_id: 'n-intro', expect: { hash: moved.hash },
    node: { type: 'paragraph', attrs: { block_id: 'n-intro' }, content: [{ type: 'text', text: '这一段被整体改写了。' }] } }])).status).toBe('accepted')
  await expect(item.getByTestId('aiws-anchor-state')).toHaveText('精确位置已失效，显示在所在块')
  await expect(prose.locator('[data-block-id="n-intro"].aiws-anno-block')).toBeVisible()
  await expect(highlight).toHaveCount(0)
  await expect(card.getByTestId('aiws-anno-card-hint')).toHaveText('精确位置已失效')

  // the paragraph is deleted: still listed, shown on the document, its quote kept
  const rewritten = await block(api, ws, 'n-intro')
  expect((await api.commit(ALICE, ws, [{ op: 'richtext.delete_blocks', entity_id: 'notes', blocks: [{ block_id: 'n-intro', expect: rewritten }] }])).status).toBe('accepted')
  await expect(item.getByTestId('aiws-anchor-state')).toHaveText('原位置已删除，显示在对象上')
  await expect(item).toContainText('文档格式')
  await expect(card.getByTestId('aiws-anno-card-hint')).toHaveText('原位置已不存在')
})

test('an application range kind: captured and located by the application, kept verbatim by the backend', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `app anchor ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  const frame = page.getByTestId('aiws-cell-frame-cell-notes')
  const prose = page.getByTestId('aiws-richtext-notes')
  await expect(prose).toBeVisible()

  // an "application" registers its own range kind: the n-th English word of a block
  await page.evaluate(() => {
    type Node = { attrs: Record<string, unknown>; isTextblock: boolean; content: { size: number }; textBetween(from: number, to: number, block?: string, leaf?: string): string }
    type Host = { entityId: string; doc: { resolve(pos: number): { parent: Node; parentOffset: number } }; selection: { from: number; to: number }; blocks: Map<string, { pos: number; node: Node }> }
    const hooks = (window as unknown as { __aiwsTestHooks: { richTextAnchors: { register(adapter: unknown): () => void } } }).__aiwsTestHooks
    const words = (text: string) => [...text.matchAll(/[A-Za-z]+/g)].map((m) => ({ start: m.index ?? 0, end: (m.index ?? 0) + m[0].length }))
    const blockText = (node: Node) => node.textBetween(0, node.content.size, undefined, '\uFFFC')
    const off = hooks.richTextAnchors.register({
      kind: 'e2e.demo/word',
      capture(host: Host) {
        if (host.selection.from !== host.selection.to) return null
        const $pos = host.doc.resolve(host.selection.from)
        const blockId = $pos.parent.attrs.block_id
        if (!$pos.parent.isTextblock || typeof blockId !== 'string') return null
        const index = words(blockText($pos.parent)).findIndex((w) => $pos.parentOffset >= w.start && $pos.parentOffset <= w.end)
        if (index < 0) return null
        return { target: { entity_id: host.entityId, selector: { kind: 'richtext_block', block_id: blockId } },
          range: { kind: 'e2e.demo/word', block_id: blockId, index }, context: { label: `第 ${index + 1} 个英文词` }, label: `第 ${index + 1} 个英文词` }
      },
      locate(host: Host, mark: { payload: { range: { block_id: string; index: number } } }) {
        const at = host.blocks.get(mark.payload.range.block_id)
        const word = at ? words(blockText(at.node))[mark.payload.range.index] : undefined
        return at && word ? { type: 'text', from: at.pos + 1 + word.start, to: at.pos + 1 + word.end } : null
      },
    })
    ;(window as unknown as { __offDemo: () => void }).__offDemo = off
  })

  // the caret inside "Engine": the application's adapter claims the selection
  const engine = prose.locator('[data-block-id="n-intro"] em', { hasText: 'Command Engine' })
  const size = await engine.boundingBox()
  await engine.click({ position: { x: (size?.width ?? 10) - 6, y: (size?.height ?? 10) / 2 } })
  await frame.getByTestId('aiws-annotate').click()
  await expect(page.getByTestId('aiws-annotation-draft-target')).toHaveText('批注对象：第 2 个英文词')
  await page.getByLabel('批注内容').fill('应用自己定位')
  await page.getByTestId('aiws-annotation-save').click()
  const item = page.getByTestId('aiws-annotation').filter({ hasText: '应用自己定位' })
  await expect(item.getByTestId('aiws-anchor-state')).toHaveText('锚点有效（由应用定位）')
  await expect(prose.locator('.aiws-anno-range')).toHaveText('Engine')
  expect(await annotation(api, ws, '应用自己定位')).toMatchObject({
    payload: { range: { kind: 'e2e.demo/word', block_id: 'n-intro', index: 1 } },
    anchor: { state: 'resolved', level: 'target', range_status: 'unchecked' },
  })

  // without the application the annotation falls back to its block
  await page.evaluate(() => (window as unknown as { __offDemo: () => void }).__offDemo())
  await expect(prose.locator('.aiws-anno-range')).toHaveCount(0)
  await expect(prose.locator('[data-block-id="n-intro"].aiws-anno-block')).toBeVisible()
  await expect(frame.getByTestId('aiws-anno-card').filter({ hasText: '应用自己定位' }).getByTestId('aiws-anno-card-hint')).toHaveText('精确位置已失效')
})
