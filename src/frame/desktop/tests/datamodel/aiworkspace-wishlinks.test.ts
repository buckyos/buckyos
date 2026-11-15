/* Wish lines and ghosts of a free canvas (许愿格交互升级 TODO §3.9; src/app/aiworkspace/ui/canvas/wishLinks): the plan from a
 * wish's data and the layout, and the routing of a line.
 *
 *   deno test --no-check --allow-env --allow-read --node-modules-dir=manual --unstable-sloppy-imports \
 *     --config tests/datamodel/aiworkspace-ink-book.deno.json tests/datamodel/aiworkspace-wishlinks.test.ts */

import type { EntityEnvelope, WishInput, WishResultBinding } from '../../src/app/aiworkspace/api/types.ts'
import type { Laid } from '../../src/app/aiworkspace/ui/canvas/layout.ts'
import { blocksBySource, ghostKey, ghostRect, GHOST_OFFSET, GHOST_SIZE, planWishLinks } from '../../src/app/aiworkspace/ui/canvas/wishLinks/plan.ts'
import { INPUT_STROKE, paintLink, RESULT_STROKE, routeLink } from '../../src/app/aiworkspace/ui/canvas/wishLinks/route.ts'

function equal(a: unknown, b: unknown, what = '') { if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error(`${what}${JSON.stringify(a)} !== ${JSON.stringify(b)}`) }
function ok(a: unknown, message: string) { if (!a) throw new Error(message) }

interface Box { x: number; y: number; w: number; h: number }
const entity = (id: string, extra: Partial<EntityEnvelope> = {}) => ({ entity_id: id, type_id: 'buckyos.cell', deleted: false, capabilities: ['read'], ...extra }) as unknown as EntityEnvelope
const block = (id: string, rect: Box, extra: Partial<EntityEnvelope> = {}, parentId = 'sf', more: Partial<Laid> = {}): [string, Laid] =>
  [id, { entity: entity(id, extra), rect, rotation: 0, bounds: rect, depth: 1, parentId, isGroup: false, paint: 0, locked: false, ...more } as Laid]
const group = (id: string, rect: Box): [string, Laid] => [id, { entity: entity(id, { type_id: 'buckyos.container' }), rect, rotation: 0, bounds: rect, depth: 1, parentId: 'sf', isGroup: true, paint: 0, locked: false } as Laid]

const WISH = { x: 1000, y: 300, w: 420, h: 300 }
const data = (inputs: WishInput[], results: WishResultBinding[] = []) => ({ cellId: 'blk-wish', inputs, results })
const input = (entity_id: string, extra: Partial<WishInput> = {}): WishInput => ({ entity_id, ...extra })
const known = (ids: string[], extra: Record<string, Partial<EntityEnvelope>> = {}) => (id: string) => (ids.includes(id) ? entity(id, { type_id: 'buckyos.table-source', title: `T ${id}`, ...extra[id] }) : undefined)

Deno.test('inputs: the nearest Block of the input, a table view input from its view, a ghost without a Block', () => {
  const laid = new Map([
    block('blk-wish', WISH, { view_type: 'wish', source_id: 'w' }),
    block('far', { x: 0, y: 1400, w: 300, h: 200 }, { source_id: 'sales' }),
    block('near', { x: 500, y: 300, w: 400, h: 300 }, { source_id: 'sales' }),
    block('view', { x: 0, y: 0, w: 400, h: 200 }, { source_id: 'orders' }),
    block('other-view', { x: 600, y: 0, w: 300, h: 200 }, { source_id: 'orders' }),
  ])
  const plan = planWishLinks(data([input('sales'), input('orders', { selector: { kind: 'table_view', cell_id: 'view' } as never }), input('notes')]), laid, blocksBySource(laid), known(['sales', 'orders', 'notes']), false)
  equal(plan.links.map((l) => [l.kind, l.from, l.to, l.entityId]), [['input', 'near', 'blk-wish', 'sales'], ['input', 'view', 'blk-wish', 'orders'], ['input', ghostKey('notes'), 'blk-wish', 'notes']])
  equal(plan.ghosts.map((g) => [g.key, g.slot, g.state]), [[ghostKey('notes'), 0, 'ok']])
  equal(ghostRect(WISH, 1), { x: WISH.x - GHOST_OFFSET - GHOST_SIZE, y: WISH.y + 84, w: GHOST_SIZE, h: GHOST_SIZE })
})

Deno.test('inputs: unreadable or deleted inputs are ghosts with no entity, even with a Block here; a table view elsewhere is a ghost', () => {
  const laid = new Map([block('blk-wish', WISH, { view_type: 'wish', source_id: 'w' }), block('secret-view', { x: 0, y: 0, w: 300, h: 200 }, { source_id: 'secret' })])
  const plan = planWishLinks(data([input('secret'), input('gone'), input('t', { selector: { kind: 'table_view', cell_id: 'elsewhere' } as never })]), laid, blocksBySource(laid),
    known(['gone', 't'], { gone: { deleted: true } }), false)
  equal(plan.links.map((l) => l.from), [ghostKey('secret'), ghostKey('gone'), ghostKey('t')])
  equal(plan.ghosts.map((g) => [g.state, Boolean(g.entity)]), [['unavailable', false], ['unavailable', false], ['ok', true]])
})

Deno.test('ghosts: more than four fold into +N; expanded shows all and a "fewer" square without a line', () => {
  const laid = new Map([block('blk-wish', WISH, { view_type: 'wish', source_id: 'w' })])
  const ids = ['a', 'b', 'c', 'd', 'e', 'f']
  const folded = planWishLinks(data(ids.map((id) => input(id))), laid, blocksBySource(laid), known(ids), false)
  equal(folded.ghosts.map((g) => [g.key, g.slot]), [[ghostKey('a'), 0], [ghostKey('b'), 1], [ghostKey('c'), 2], ['@more', 3]])
  equal(folded.ghosts[3].folded?.map((g) => g.entityId), ['d', 'e', 'f'])
  equal(folded.links.map((l) => l.from), [ghostKey('a'), ghostKey('b'), ghostKey('c'), '@more'])
  const open = planWishLinks(data(ids.map((id) => input(id))), laid, blocksBySource(laid), known(ids), true)
  equal(open.ghosts.map((g) => g.key).at(-1), '@less')
  equal(open.links.length, 6)
  const four = planWishLinks(data(ids.slice(0, 4).map((id) => input(id))), laid, blocksBySource(laid), known(ids), false)
  equal(four.ghosts.map((g) => g.state), ['ok', 'ok', 'ok', 'ok'])
})

Deno.test('results: each Block here; none for a result elsewhere; a group of more than six as one line', () => {
  const seven = Array.from({ length: 7 }, (_, i) => block(`g7-${i}`, { x: 1500 + i * 10, y: 300, w: 100, h: 100 }, {}, 'grp7'))
  const six = Array.from({ length: 6 }, (_, i) => block(`g6-${i}`, { x: 1500 + i * 10, y: 900, w: 100, h: 100 }, {}, 'grp6'))
  const laid = new Map([block('blk-wish', WISH, { view_type: 'wish', source_id: 'w' }), group('grp7', { x: 1480, y: 260, w: 400, h: 300 }), group('grp6', { x: 1480, y: 860, w: 400, h: 300 }), ...seven, ...six,
    block('loose', { x: 1500, y: 1300, w: 100, h: 100 })])
  const results: WishResultBinding[] = [
    { type: 'image', entity_id: 'r7', cells: seven.map(([id]) => id) },
    { type: 'image', entity_id: 'r6', cells: six.map(([id]) => id) },
    { type: 'record', entity_id: 'r1', cells: ['loose', 'on-another-canvas'] },
    { type: 'table', entity_id: 'r0', cells: ['on-another-canvas'] },
  ]
  const plan = planWishLinks(data([], results), laid, blocksBySource(laid), known([]), false)
  const lines = plan.links.map((l) => [l.to, l.count ?? null])
  equal(lines[0], ['grp7', 7])
  equal(lines.slice(1, 7).map((l) => l[0]), six.map(([id]) => id))
  equal(lines.at(-1), ['loose', null])
  equal(plan.links.length, 8)
  ok(plan.links.every((l) => l.kind === 'result' && l.from === 'blk-wish'), 'result lines leave the wish')
})

Deno.test('results: a control is its own Block; derived columns pair with the input line of the same table Block', () => {
  const laid = new Map([block('blk-wish', WISH, { view_type: 'wish', source_id: 'w' }), block('tbl', { x: 300, y: 300, w: 500, h: 300 }, { source_id: 'sales' }),
    block('ctl', { x: 1600, y: 300, w: 300, h: 300 }, { type_id: 'buckyos.container' })])
  const plan = planWishLinks(data([input('sales')], [{ type: 'table_columns', entity_id: 'sales' }, { type: 'control', entity_id: 'ctl' }]), laid, blocksBySource(laid), known(['sales']), false)
  equal(plan.links.map((l) => [l.kind, l.from, l.to, l.paired]), [['input', 'tbl', 'blk-wish', true], ['result', 'blk-wish', 'tbl', true], ['result', 'blk-wish', 'ctl', false]])
  // read through a table view: the columns are drawn to that view, not to a nearer Block of the same table
  const views = new Map([...laid, block('near-tbl', { x: 900, y: 650, w: 200, h: 100 }, { source_id: 'sales' })])
  const viaView = planWishLinks(data([input('sales', { selector: { kind: 'table_view', cell_id: 'tbl' } as never })], [{ type: 'table_columns', entity_id: 'sales' }]), views, blocksBySource(views), known(['sales']), false)
  equal(viaView.links.map((l) => [l.kind, l.kind === 'input' ? l.from : l.to, l.paired]), [['input', 'tbl', true], ['result', 'tbl', true]])
})

Deno.test('routing: facing sides, the arrow at the far end; a derived column runs apart (blue above, orange below)', () => {
  const left = { rect: { x: 0, y: 0, w: 200, h: 100 }, rotation: 0, shape: 'rect' as const }
  const right = { rect: { x: 400, y: 0, w: 200, h: 100 }, rotation: 0, shape: 'rect' as const }
  const g = routeLink(left, right, false)
  equal([g.start.point.x, g.start.point.y, g.end.point.x, g.end.point.y], [200, 50, 400, 50])
  equal(g.route, 'curve')
  const blue = routeLink(left, right, true)
  const orange = routeLink(right, left, true)
  ok(blue.start.point.y < 50 && blue.end.point.y < 50, `blue runs above: ${JSON.stringify([blue.start.point, blue.end.point])}`)
  ok(orange.start.point.y > 50 && orange.end.point.y > 50, `orange runs below: ${JSON.stringify([orange.start.point, orange.end.point])}`)
  equal([blue.start.point.x, blue.end.point.x, orange.start.point.x, orange.end.point.x], [200, 400, 400, 200])
  const input = paintLink(g, 'input', 1, 't1')
  ok(input.markup.includes(`stroke="${INPUT_STROKE}"`) && input.markup.includes('stroke-dasharray'), 'a dashed blue line')
  ok(input.box.x <= 200 && input.box.x + input.box.w >= 400, 'the frame holds the line')
  ok(paintLink(g, 'result', 0.2, 't2').markup.includes(`stroke="${RESULT_STROKE}"`), 'an orange line')
})
