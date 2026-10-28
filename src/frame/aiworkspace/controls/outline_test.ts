/* Markdown outline and OPML conversion (脑图 TODO M1/M3/M4): the cases in fixtures/mindmap/outlines.json are shared
 * with the service's Rust parser (store wish results), so a wish program and the model land the same tree;
 * `MINDMAP_REGEN=1 deno test -A` rewrites their expected trees after an intended change. */

import { assert, assertEquals } from './test_assert.ts'
import { formatInline, parseInline, plainText } from './inline.ts'
import { controlOutline, formatMarkdownOutline, mindmapOperations, nodeOperations, nodeTitle, parseMarkdownOutline, readerOf, type OutlineNode } from './outline.ts'
import { formatOpml, parseOpml } from './opml.ts'

const here = new URL('.', import.meta.url).pathname
const CASES = `${here}../fixtures/mindmap/outlines.json`
const REGEN = Deno.env.get('MINDMAP_REGEN') === '1'

interface Case { name: string; markdown: string; tree: OutlineNode[] }

Deno.test('outline cases shared with the service', () => {
  const file = JSON.parse(Deno.readTextFileSync(CASES)) as { description: string; cases: Case[] }
  if (REGEN) {
    for (const c of file.cases) c.tree = parseMarkdownOutline(c.markdown)
    Deno.writeTextFileSync(CASES, JSON.stringify(file, null, 1) + '\n')
    return
  }
  for (const c of file.cases) assertEquals(parseMarkdownOutline(c.markdown), c.tree, c.name)
})

Deno.test('a tree reads back as the outline it came from', () => {
  const md = '- Plan\n  - **Market**\n    - [Interviews](https://example.com/a)\n  - Product\n'
  const tree = parseMarkdownOutline(md)
  assertEquals(formatMarkdownOutline(tree), md)
  const ids = (() => { let n = 0; return (p: string) => `${p}${++n}` })()
  const built = mindmapOperations(tree, { parentId: 's1', orderKey: 'a', at: { x: 10, y: 20 }, newId: ids })
  assertEquals(built.ops.length, 5)
  assertEquals((built.ops[0].payload as { kind: string }).kind, 'mindmap')
  assertEquals(built.ops[0].placement, { x: 10, y: 20, w: 'auto', h: 'auto' })
  for (const op of built.ops.slice(1)) assertEquals(op.placement, { x: 'auto', y: 'auto', w: 'auto', h: 'auto' })
  const outline = built.ops.map((op) => ({ entity_id: op.entity_id as string, type_id: op.type_id as string, parent_id: op.parent_id as string, order_key: op.order_key as string,
    kind: (op.payload as { kind?: string }).kind, view_type: (op.payload as { view?: { type: string } }).view?.type, title: (op.payload as { title?: string }).title }))
  assertEquals(controlOutline(readerOf(outline), built.containerId), tree)
})

Deno.test('several top-level items get one root when asked', () => {
  const tree = parseMarkdownOutline('- a\n- b\n')
  assertEquals(mindmapOperations(tree, { parentId: 's', orderKey: 'a' }).rootIds.length, 2)
  assertEquals(mindmapOperations(tree, { parentId: 's', orderKey: 'a', rootText: 'Topic' }).rootIds.length, 1)
  const sub = nodeOperations(tree, 'n0')
  assertEquals(sub.ids.length, 2)
  assert(sub.ops.every((op) => op.parent_id === 'n0'), 'children of the given node')
})

Deno.test('inline formatting survives, titles stay within the core limit', () => {
  const src = '**bold** and *it* `code` ~~gone~~ [link](https://x.y/z) a\\*b'
  const runs = parseInline(src)
  assertEquals(plainText(src), 'bold and it code gone link a*b')
  assertEquals(parseInline(formatInline(runs)), runs)
  assert(runs.some((r) => r.bold && r.text === 'bold') && runs.some((r) => r.href === 'https://x.y/z'), 'styles')
  assertEquals([...nodeTitle('字'.repeat(300))].length, 256)
  assertEquals(plainText('snake_case_name'), 'snake_case_name')
})

Deno.test('OPML round trip', () => {
  const tree = parseMarkdownOutline('- Root\n  - A & B\n    - <c>\n  - D\n')
  const xml = formatOpml(tree, 'Title')
  const back = parseOpml(xml)
  assertEquals(back.title, 'Title')
  assertEquals(back.nodes, tree)
  let failed = false
  try { parseOpml('<html/>') } catch { failed = true }
  assert(failed, 'not OPML')
})
