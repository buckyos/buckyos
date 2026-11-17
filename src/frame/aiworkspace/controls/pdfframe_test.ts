/* PDF Frames (打印 TODO §3.3, T3): the page shapes and orientations, the box keeping its shape whatever height is stored,
 * the bands and the content area, a rescale that keeps the frame's scale, and the operations of a new frame. Run:
 * `deno test -A` in this folder. */

import { assert, assertEquals } from './test_assert.ts'
import { approxMetrics, NodeMeasurer } from './measure.ts'
import { placeControl, solveAny, type ControlEntity, type ControlSource } from './solve.ts'
import { DEFAULT_FRAME_CONFIG, frameAreas, frameBox, frameRatio, pdfFrameConfig, pdfFrameOperations, rescaledWidth } from './pdfframe.ts'

const measurer = new NodeMeasurer(approxMetrics)
const close = (a: number, b: number, eps = 0.01) => Math.abs(a - b) <= eps

function source(entities: (ControlEntity & { parent_id?: string; order_key?: string })[]): ControlSource {
  return {
    get: (id) => entities.find((e) => e.entity_id === id),
    childrenOf: (id) => entities.filter((e) => e.parent_id === id).sort((a, b) => ((a.order_key ?? '') < (b.order_key ?? '') ? -1 : 1)),
  }
}

Deno.test('the page shapes: A (A3 / A4 / A5 alike), Letter, 16:9, 4:3, custom; the orientation turns them', () => {
  assert(close(frameRatio(pdfFrameConfig({ page: 'A' })), 210 / 297, 0.001))
  assert(close(frameRatio(pdfFrameConfig({ page: 'A', orientation: 'landscape' })), 297 / 210, 0.001))
  assert(close(frameRatio(pdfFrameConfig({ page: 'Letter' })), 215.9 / 279.4, 0.001))
  // slides are landscape unless said otherwise
  assert(close(frameRatio(pdfFrameConfig({ page: '16:9' })), 16 / 9, 0.001))
  assert(close(frameRatio(pdfFrameConfig({ page: '4:3', orientation: 'portrait' })), 3 / 4, 0.001))
  assert(close(frameRatio(pdfFrameConfig({ page: 'custom', custom_w: 100, custom_h: 50 })), 2, 0.001))
  assert(close(frameRatio(pdfFrameConfig({ page: 'custom', custom_w: 100, custom_h: 50, orientation: 'portrait' })), 0.5, 0.001))
  // what is not valid takes the default
  assertEquals(pdfFrameConfig({ page: 'B5', header: -1, footer: 'x', margin: 1e9, custom_w: 0 }), DEFAULT_FRAME_CONFIG)
})

Deno.test('the box keeps its shape: the height follows the width; the bands and the content area fit in it', () => {
  const c = pdfFrameConfig({ page: 'A', header: 40, footer: 30, margin: 20 })
  const box = frameBox(420, c)
  assertEquals(Math.round(box.h), 594)
  // no width: the natural one (one unit a point of an A4)
  assertEquals(Math.round(frameBox(undefined, c).w), 595)
  const areas = frameAreas(box, c)
  assertEquals(areas.header, { x: 0, y: 0, w: 420, h: 40 })
  assertEquals(Math.round(areas.footer.y), 564)
  assertEquals(areas.content.x, 20)
  assertEquals(areas.content.w, 380)
  assertEquals(Math.round(areas.content.h), 524)
  // bands of 0, and bands higher than the page: never past the box, the content area empty
  const none = frameAreas(box, pdfFrameConfig({ header: 0, footer: 0, margin: 0 }))
  assertEquals(none.content, { x: 0, y: 0, w: 420, h: box.h })
  const tall = frameAreas({ w: 100, h: 100 }, pdfFrameConfig({ header: 80, footer: 80, margin: 80 }))
  assertEquals([tall.header.h, tall.footer.h, tall.footer.y, tall.content.h, tall.content.w], [80, 20, 80, 0, 0])
})

Deno.test('a frame is laid out by its page: its bands full width at the top and the bottom, whatever height is stored', () => {
  const src = source([
    { entity_id: 'pf', type_id: 'buckyos.container', kind: 'pdf-frame', config: { page: 'A', orientation: 'landscape', header: 50, footer: 40 }, placement: { x: 10, y: 20, w: 842, h: 100 } },
    { entity_id: 'h', type_id: 'buckyos.container', kind: 'group', slot: 'header', parent_id: 'pf', order_key: 'a', placement: { x: 5, y: 5, w: 1, h: 1 } },
    { entity_id: 'f', type_id: 'buckyos.container', kind: 'group', slot: 'footer', parent_id: 'pf', order_key: 'b', placement: { x: 5, y: 5, w: 1, h: 1 } },
    { entity_id: 'x', type_id: 'buckyos.cell', view_type: 'label', parent_id: 'pf', order_key: 'c', placement: { x: 0, y: 0, w: 10, h: 10 } },
  ])
  const solved = solveAny(src, 'pf', { measurer })
  assertEquals(Math.round(solved.layout.bounds.h), 595)
  assertEquals([...solved.layout.nodes.keys()], ['h', 'f'])
  const placed = placeControl(solved, { x: 10, y: 20 })
  assertEquals(placed.nodes.get('h')!.y, 20)
  assertEquals(placed.nodes.get('h')!.w, 842)
  assertEquals(Math.round(placed.nodes.get('f')!.y), 20 + 595 - 40)
  assertEquals(placed.controls.get('pf')!.box.x, 10)
})

Deno.test('turning or reshaping a frame keeps its scale; a new frame comes with its two bands', () => {
  const portrait = pdfFrameConfig({ page: 'A' })
  const landscape = pdfFrameConfig({ page: 'A', orientation: 'landscape' })
  assertEquals(rescaledWidth(595, portrait, landscape), 842)
  assertEquals(rescaledWidth(1190, portrait, landscape), 1684)
  assertEquals(rescaledWidth(842, landscape, pdfFrameConfig({ page: '16:9' })), 842)
  const ops = pdfFrameOperations({ id: 'pf', parentId: 's', orderKey: 'm', x: 10.4, y: 20, headerId: 'pf-h', footerId: 'pf-f' }) as { payload: Record<string, unknown>; placement: Record<string, number> }[]
  assertEquals(ops.length, 3)
  assertEquals(ops[0].placement, { x: 10, y: 20, w: 595, h: 842 })
  assertEquals(ops.slice(1).map((o) => o.payload.slot), ['header', 'footer'])
  assertEquals(ops[2].placement, { x: 0, y: 786, w: 595, h: 56 })
  // a slide shape comes landscape unless said otherwise
  const slide = pdfFrameOperations({ id: 'pf', parentId: 's', orderKey: 'm', x: 0, y: 0, config: { page: '16:9' }, headerId: 'h', footerId: 'f' }) as { payload: Record<string, unknown>; placement: Record<string, number> }[]
  assertEquals(slide[0].placement, { x: 0, y: 0, w: 842, h: 474 })
  assertEquals((slide[0].payload.config as Record<string, unknown>).orientation, 'landscape')
})
