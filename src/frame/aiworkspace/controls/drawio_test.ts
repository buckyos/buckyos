/* A mind map for draw.io (脑图 TODO M4): every control a group at its box, every node a shape at its laid-out place,
 * every implicit edge a real line bound to both nodes — run through the shared draw.io exporter. */

import { assert, assertEquals } from './test_assert.ts'
import { exportDiagram, type ExportNode } from '../import-drawio/mod.ts'
import { exportControl, nodeShapeConfig } from './drawio.ts'
import { approxMetrics, NodeMeasurer } from './measure.ts'
import { replay } from './testing.ts'

const here = new URL('.', import.meta.url).pathname
const src = replay(JSON.parse(Deno.readTextFileSync(`${here}../fixtures/mindmap/commits.json`)).commits)

Deno.test('a map with a nested map becomes groups, shapes and bound lines', () => {
  const out = exportControl(src, 'mm-min', new NodeMeasurer(approxMetrics), null, { x: 0, y: 0 })
  assertEquals(out.groups.map((g) => g.id).sort(), ['min-inner', 'mm-min'])
  assertEquals(out.groups.find((g) => g.id === 'min-inner')!.parent, 'mm-min')
  // the outer map: 11 nodes, the inner map one of them (a group); 3 inside it — 10 + 2 edges
  assertEquals(out.nodes.length, 10 + 3)
  assertEquals(out.edges.length, 10 + 2)
  for (const e of out.edges) assert(['e', 'w', 's', 'n'].includes(e.fromAnchor) && ['e', 'w', 's', 'n'].includes(e.toAnchor), e.id)
  const nodes: ExportNode[] = [
    ...out.groups.map((g): ExportNode => ({ id: g.id, parent: g.parent, kind: 'group', placement: g.placement })),
    ...out.nodes.map((n): ExportNode => ({ id: n.id, parent: n.control, kind: 'cell', view: 'shape', placement: n.placement, config: nodeShapeConfig(n.depth, n.color), title: src.get(n.id)?.title ?? '' })),
    ...out.edges.map((e): ExportNode => ({ id: e.id, parent: e.control, kind: 'cell', view: 'connector', placement: e.placement, config: { stroke: e.color, width: e.width, end_cap: 'none' },
      line: { start: { entity_id: e.from, anchor: { kind: 'named', id: e.fromAnchor } }, end: { entity_id: e.to, anchor: { kind: 'named', id: e.toAnchor } }, route: e.route } })),
  ]
  const { xml } = exportDiagram(nodes, { name: 'mind' })
  // every line keeps both ends
  const edges = [...xml.matchAll(/<mxCell [^>]*edge="1"[^>]*>/g)].map((m) => m[0])
  assertEquals(edges.length, 12)
  assert(edges.every((e) => /source="/.test(e) && /target="/.test(e)), 'bound at both ends')
})
