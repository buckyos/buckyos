/* Composite layouts (扩展控件 P1, K2): stack (column / row, slot order, title band, spacing from config), fixed,
 * pinned children, nested controls, the accept rule. Run: `deno test -A` in this folder. */

import { assert, assertEquals } from './test_assert.ts'
import { approxMetrics, NodeMeasurer } from './measure.ts'
import { placeControl, solveAny, type ControlEntity, type ControlSource } from './solve.ts'
import { slotAccepts, slotFor, templated, TITLE_BAND, type Composition } from './composite.ts'

const measurer = new NodeMeasurer(approxMetrics)

const board: Composition = {
  layout: { algorithm: 'stack', direction: 'column', spacing: '{{config.gap}}', padding: 10 },
  slots: { header: { max: 1, accepts: ['richtext'], placement: 'top' }, cards: { accepts: ['cell:did:bns:alice#card'] }, footer: { placement: 'bottom' } },
  chrome: { title: { text: '{{config.title}} ({{config.gap}})', slot: 'header' } },
}

function source(entities: (ControlEntity & { parent_id?: string; order_key?: string })[]): ControlSource {
  return {
    get: (id) => entities.find((e) => e.entity_id === id),
    childrenOf: (id) => entities.filter((e) => e.parent_id === id).sort((a, b) => ((a.order_key ?? '') < (b.order_key ?? '') ? -1 : 1)),
  }
}

const def = (composition: Composition): ControlEntity => ({ entity_id: 'd-board', type_id: 'buckyos.block-def', composition, config_defaults: { gap: 8, title: 'Today' } })
const card = (id: string, key: string, slot: string, extra: Partial<ControlEntity> = {}) =>
  ({ entity_id: id, type_id: 'buckyos.cell', parent_id: 'b1', order_key: key, view_type: 'html', def_id: 'd-card', slot, placement: { x: 'auto', y: 'auto', w: 200, h: 60 }, ...extra } as ControlEntity & { parent_id: string; order_key: string })

Deno.test('a stack lays its slots out in order, below the title, spaced by config', () => {
  const src = source([def(board), { entity_id: 'b1', type_id: 'buckyos.container', kind: 'composite', def_id: 'd-board', config: { gap: 4 } },
    card('c2', 'b', 'cards'), card('f', 'a', 'footer'), card('c1', 'a', 'cards')])
  const s = solveAny(src, 'b1', { measurer })
  const at = (id: string) => s.layout.nodes.get(id)!
  assertEquals([at('c1').x, at('c1').y], [10, 10 + TITLE_BAND], 'the title band takes the top while the header slot is empty')
  assertEquals(at('c2').y, at('c1').y + 60 + 4, 'spacing from the instance config')
  assertEquals(at('f').y, at('c2').y + 60 + 4, 'bottom slots come last')
  assertEquals(s.layout.bounds, { x: 0, y: 0, w: 220, h: at('f').y + 60 + 10 })
  assertEquals(templated(board.chrome!.title!.text, { title: 'Sprint', gap: 4 }), 'Sprint (4)')
  assertEquals(templated('{{config.gap}}', { gap: 4 }), 4)
})

Deno.test('a row, a pinned child, a header filling the title slot, an empty box', () => {
  const row: Composition = { ...board, layout: { algorithm: 'stack', direction: 'row', spacing: 6, padding: { t: 2, r: 3, b: 4, l: 5 } } }
  const src = source([def(row), { entity_id: 'b1', type_id: 'buckyos.container', kind: 'composite', def_id: 'd-board' },
    card('h', 'a', 'header', { view_type: 'richtext', def_id: null, placement: { x: 'auto', y: 'auto', w: 100, h: 30 } }), card('c1', 'b', 'cards'), card('p', 'c', 'cards', { placement: { x: 500, y: 500, w: 10, h: 10 } })])
  const s = solveAny(src, 'b1', { measurer })
  const at = (id: string) => s.layout.nodes.get(id)!
  assertEquals([at('h').x, at('h').y], [5, 2], 'no title band once the header slot holds something')
  assertEquals(at('c1').x, 5 + 100 + 6)
  assert(at('p').pinned && at('p').x === 500, 'numeric x / y are kept')
  const empty = solveAny(source([def(board), { entity_id: 'b2', type_id: 'buckyos.container', kind: 'composite', def_id: 'd-board' }]), 'b2', { measurer })
  assert(empty.layout.bounds.w >= 200 && empty.layout.bounds.h >= 100, 'room for the empty text')
})

Deno.test('fixed keeps every place; nested controls are solved and placed inside', () => {
  const fixed: Composition = { layout: { algorithm: 'fixed', padding: 0 }, slots: { area: { placement: 'free' } } }
  const src = source([def(fixed), { entity_id: 'b1', type_id: 'buckyos.container', kind: 'composite', def_id: 'd-board' },
    card('a', 'a', 'area', { placement: { x: 40, y: 30, w: 50, h: 50 } }), card('auto', 'b', 'area'),
    { entity_id: 'inner', type_id: 'buckyos.container', kind: 'composite', def_id: 'd-board', parent_id: 'b1', order_key: 'c', slot: 'area', placement: { x: 200, y: 0, w: 'auto', h: 'auto' } } as ControlEntity])
  const s = solveAny(src, 'b1', { measurer })
  assertEquals([s.layout.nodes.get('a')!.x, s.layout.nodes.get('a')!.y], [40, 30])
  assert(s.layout.nodes.get('auto')!.y >= 80, 'an "auto" child falls below the placed ones')
  assert(s.nested.has('inner'))
  const placed = placeControl(s, { x: 1000, y: 1000 })
  assertEquals(placed.controls.get('inner')!.box.x, 1200)
})

Deno.test('what a slot accepts', () => {
  const ident = (id: string) => (id === 'd-card' ? 'did:bns:alice#card' : null)
  const cell = { type_id: 'buckyos.cell', view_type: 'html', def_id: 'd-card' }
  assert(slotAccepts(board.slots.cards, cell, ident))
  assert(!slotAccepts(board.slots.header, cell, ident))
  assert(slotAccepts(board.slots.footer, { type_id: 'buckyos.container' }, ident), 'no accepts: any Block')
  assertEquals(slotFor(board, { type_id: 'buckyos.cell', view_type: 'richtext' }, ident), 'header')
  assertEquals(slotFor(board, { type_id: 'buckyos.cell', view_type: 'richtext' }, ident, (s) => (s === 'header' ? 1 : 0)), 'footer', 'a full slot is skipped')
})
