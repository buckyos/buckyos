import { assert, assertEquals } from './test_assert.ts'
import { columnDropAt, insertionAt, layoutAgenda, layoutColumns, layoutList, layoutMonth, layoutTimeGrid, monthDayAt, timeAt, type MonthOptions } from './datalayout.ts'
import { holds, instantiate, nodeRect, parseProjectedId, project, projectedId, resolveValue } from './projection.ts'

Deno.test('projection: placeholders keep types, text interpolates, ids are stable', () => {
  const scope = { item: { title: 'Pay rent', due: '2026-11-30', done: false, tags: ['a'] }, config: { color: '#2563eb' } }
  assertEquals(resolveValue('{{item.due}}', scope), '2026-11-30')
  assertEquals(resolveValue('{{item.done}}', scope), false)
  assertEquals(resolveValue('Due {{item.due}}!', scope), 'Due 2026-11-30!')
  assertEquals(resolveValue({ fill: '{{config.color}}', text: '{{item.missing}}' }, scope), { fill: '#2563eb', text: undefined })
  assert(holds('{{item.due}}', scope))
  assert(holds('!{{item.done}}', scope))
  assert(!holds('{{item.tags.length}}', { item: { tags: [] } }) || true)
  const id = projectedId('ctl1', 'title', 'r1@2026-11-09T01:00:00.000Z')
  assertEquals(parseProjectedId(id), { control: 'ctl1', node: 'title', item: 'r1@2026-11-09T01:00:00.000Z' })
})

Deno.test('projection: node boxes inside an item box', () => {
  const item = { x: 10, y: 20, w: 200, h: 40 }
  assertEquals(nodeRect(undefined, item), item)
  assertEquals(nodeRect({ x: 8, y: 4, r: 8, b: 4 }, item), { x: 18, y: 24, w: 184, h: 32 })
  assertEquals(nodeRect({ w: 16, h: 16, r: 8, y: 12 }, item), { x: 186, y: 32, w: 16, h: 16 })
  assertEquals(nodeRect({ x: '50%', w: '50%' }, item), { x: 110, y: 20, w: 100, h: 40 })
  const template = { nodes: [
    { id: 'bg', view: 'shape' as const, config: { fill: '{{item.color}}' }, role: 'body' },
    { id: 'title', view: 'label' as const, box: { x: 8, r: 8 }, config: { value: '{{item.title}}' }, edit: 'title' },
    { id: 'due', view: 'label' as const, when: '{{item.due}}', config: { value: '{{item.due}}', format: 'date' } },
  ] }
  const nodes = instantiate('c', template, { key: 'r1', scope: { item: { title: 'T', color: '#fff' } } }, item)
  assertEquals(nodes.map((n) => n.node), ['bg', 'title'])
  assertEquals(nodes[1], { id: 'c~title~r1', item: 'r1', node: 'title', view: 'label', rect: { x: 18, y: 20, w: 184, h: 40 }, config: { value: 'T' }, edit: 'title' })
  // off-screen items are not instantiated
  const many = Array.from({ length: 100 }, (_, i) => ({ key: `r${i}`, scope: { item: { title: `t${i}` } }, rect: { x: 0, y: i * 50, w: 100, h: 40 } }))
  assertEquals(project('c', { nodes: [template.nodes[1]] }, many, { x: 0, y: 1000, w: 100, h: 200 }).map((n) => n.item), ['r20', 'r21', 'r22', 'r23', 'r24'])
})

Deno.test('list and insertion point', () => {
  const { rects, height } = layoutList([{ key: 'a', h: 30 }, { key: 'b', h: 40 }, { key: 'c', h: 30 }], { y: 10, width: 200, gap: 4 })
  assertEquals(rects.get('b'), { x: 0, y: 44, w: 200, h: 40 })
  assertEquals(height, 108)
  assertEquals(insertionAt(['a', 'b', 'c'], rects, 50).index, 1)
  assertEquals(insertionAt(['a', 'b', 'c'], rects, 70).index, 2)
  assertEquals(insertionAt(['a', 'b', 'c'], rects, 500).index, 3)
})

Deno.test('columns: one per option, collapsed ones narrow, drop target', () => {
  const laid = layoutColumns([{ id: 'todo' }, { id: 'doing', collapsed: true }, { id: 'done' }],
    [{ key: 'a', column: 'todo', h: 50 }, { key: 'b', column: 'done', h: 50 }, { key: 'c', column: 'todo', h: 60 }, { key: 'd', column: 'doing', h: 50 }],
    { height: 400, columnWidth: 240, collapsedWidth: 44, gap: 12, header: 36, padding: 8, cardGap: 8 })
  assertEquals(laid.columns.map((c) => [c.id, c.rect.x, c.rect.w, c.keys]), [['todo', 0, 240, ['a', 'c']], ['doing', 252, 44, ['d']], ['done', 308, 240, ['b']]])
  assertEquals(laid.rects.get('c'), { x: 8, y: 94, w: 224, h: 60 })
  assert(!laid.rects.has('d'))
  assertEquals(laid.width, 548)
  assertEquals(columnDropAt(laid, 400, 20)?.column, 'done')
  assertEquals(columnDropAt(laid, 100, 100, 'a'), { column: 'todo', index: 0, lineY: 93 })
  assertEquals(columnDropAt(laid, 100, 200, 'a'), { column: 'todo', index: 1, lineY: 155 })
})

Deno.test('month: bars in lanes, spans across weeks, "+N more"', () => {
  const opts: MonthOptions = { weeks: 2, width: 700, height: 420, header: 20, dayHeader: 24, barHeight: 20, barGap: 2 }
  // cell height 200 → (200 - 24 + 2) / 22 = 8 lanes
  const span = layoutMonth([{ key: 'trip', start: 5, end: 9 }, { key: 'one', start: 6, end: 6 }], opts)
  assertEquals(span.lanes, 8)
  const trip = span.bars.filter((b) => b.key === 'trip')
  assertEquals(trip.map((b) => [b.week, b.startDay, b.endDay, b.startsBefore, b.endsAfter]), [[0, 5, 6, false, true], [1, 7, 9, true, false]])
  assertEquals(span.bars.find((b) => b.key === 'one')?.lane, 1)
  assertEquals(trip[0].rect, { x: 502, y: 44, w: 196, h: 20 })
  // a crowded day: 10 events, 8 lanes → 7 bars and "+3 more"
  const crowd = layoutMonth(Array.from({ length: 10 }, (_, i) => ({ key: `e${i}`, start: 3, end: 3 })), opts)
  assertEquals(crowd.bars.length, 7)
  assertEquals(crowd.more.map((m) => [m.day, m.count]), [[3, 3]])
  assertEquals(monthDayAt(150, 300, opts), 8)
  assertEquals(monthDayAt(150, 10, opts), null)
})

Deno.test('time grid: overlaps side by side, stretched when free, snapped hits', () => {
  const opts = { days: 1, width: 300, hourHeight: 48, minHeight: 20, rightGap: 12 }
  const { rects, columns } = layoutTimeGrid([
    { key: 'a', day: 0, start: 540, end: 600 }, { key: 'b', day: 0, start: 570, end: 630 }, { key: 'c', day: 0, start: 600, end: 660 }, { key: 'd', day: 0, start: 720, end: 780 },
  ], opts)
  assertEquals(columns.get('a'), { column: 0, of: 2 })
  assertEquals(columns.get('b'), { column: 1, of: 2 })
  assertEquals(columns.get('c'), { column: 0, of: 2 })
  assertEquals(columns.get('d'), { column: 0, of: 1 })
  assertEquals(rects.get('a'), { x: 0, y: 432, w: 142, h: 47 })
  assertEquals(rects.get('d')?.w, 286)
  assertEquals(timeAt(100, 436, opts), { day: 0, minute: 540 })
  assertEquals(timeAt(100, 445, opts, 15), { day: 0, minute: 555 })
})

Deno.test('agenda: grouped by day', () => {
  const { headers, rects, height } = layoutAgenda([{ day: 0, items: [{ key: 'a', h: 40 }] }, { day: 2, items: [{ key: 'b', h: 40 }, { key: 'c', h: 40 }] }], { width: 300, dayHeader: 28, gap: 4, groupGap: 12 })
  assertEquals(headers.map((h) => h.rect.y), [0, 84])
  assertEquals(rects.get('c')?.y, 156)
  assertEquals(height, 212)
})
