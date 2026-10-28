/* Layout snapshots (脑图 TODO M1, R18: the formula is not versioned, these guard against unintended changes): the M0
 * fixture (fixtures/mindmap/commits.json) solved with the fixed-width metrics, per direction, folded, pinned and nested;
 * `MINDMAP_REGEN=1 deno test -A` rewrites fixtures/mindmap/layout/*.json after an intended change. Plus the invariants
 * the snapshots alone would not explain: no overlaps, growth on the declared side, sizes from the title.
 * Run: `deno test -A` in this folder. */

import { assert, assertEquals } from './test_assert.ts'
import { approxMetrics, NodeMeasurer } from './measure.ts'
import { placeControl, solveControl, type ControlSource, type SolvedControl } from './solve.ts'
import { branchColors, edgeStrokes } from './edges.ts'
import { layoutTree, mindmapConfig, type LaidNode, type TreeNode } from './tree.ts'
import { replay } from './testing.ts'

const here = new URL('.', import.meta.url).pathname
const FIXTURE = `${here}../fixtures/mindmap/commits.json`
const SNAPSHOTS = `${here}../fixtures/mindmap/layout/`
const REGEN = Deno.env.get('MINDMAP_REGEN') === '1'

const src = replay(JSON.parse(Deno.readTextFileSync(FIXTURE)).commits)
const measurer = new NodeMeasurer(approxMetrics)

function solve(id: string, folded: string[] = [], source: ControlSource = src): SolvedControl {
  return solveControl(source, id, { measurer, collapsed: () => new Set(folded) })
}

const r1 = (n: number) => Math.round(n * 10) / 10
function shape(s: SolvedControl): unknown {
  const b = s.layout.bounds
  return {
    bounds: { x: r1(b.x), y: r1(b.y), w: r1(b.w), h: r1(b.h) },
    nodes: [...s.layout.nodes.values()].map((n) => ({ id: n.id, x: r1(n.x), y: r1(n.y), w: r1(n.w), h: r1(n.h), depth: n.depth, grow: n.grow, ...(n.pinned ? { pinned: true } : {}), ...(n.collapsed ? { collapsed: true } : {}) })),
    ...(s.nested.size ? { nested: Object.fromEntries([...s.nested].map(([k, v]) => [k, shape(v)])) } : {}),
  }
}

function snapshot(name: string, value: unknown) {
  const path = `${SNAPSHOTS}${name}.json`
  const text = JSON.stringify(value, null, 1) + '\n'
  if (REGEN) { Deno.mkdirSync(SNAPSHOTS, { recursive: true }); Deno.writeTextFileSync(path, text); return }
  let want: string
  try { want = Deno.readTextFileSync(path) } catch { throw new Error(`missing snapshot ${path}: run MINDMAP_REGEN=1 deno test -A`) }
  assertEquals(JSON.parse(text), JSON.parse(want), `layout of ${name} changed (MINDMAP_REGEN=1 to accept)`)
}

function overlaps(nodes: LaidNode[]): string[] {
  const out: string[] = []
  for (let i = 0; i < nodes.length; i++) for (let j = i + 1; j < nodes.length; j++) {
    const a = nodes[i], b = nodes[j]
    if (a.x < b.x + b.w - 0.01 && b.x < a.x + a.w - 0.01 && a.y < b.y + b.h - 0.01 && b.y < a.y + a.h - 0.01) out.push(`${a.id}×${b.id}`)
  }
  return out
}

for (const dir of ['right', 'left', 'both', 'down']) {
  Deno.test(`layout ${dir}`, () => {
    const s = solve(`mm-${dir}`)
    snapshot(dir, shape(s))
    const nodes = [...s.layout.nodes.values()]
    assertEquals(overlaps(nodes.filter((n) => !n.pinned)), [], 'nodes overlap')
    for (const n of nodes) {
      const p = n.parent ? s.layout.nodes.get(n.parent) : undefined
      if (!p || n.pinned) continue
      if (n.grow === 'right') assert(n.x >= p.x + p.w, `${n.id} right of its parent`)
      if (n.grow === 'left') assert(n.x + n.w <= p.x, `${n.id} left of its parent`)
      if (n.grow === 'down') assert(n.y >= p.y + p.h, `${n.id} below its parent`)
    }
    const root = s.layout.nodes.get(`${dir}-root`)!
    assertEquals([root.x, root.y], [0, 0], 'the first root is the origin')
    const b = s.layout.bounds
    for (const n of nodes) assert(n.x >= b.x && n.y >= b.y && n.x + n.w <= b.x + b.w && n.y + n.h <= b.y + b.h, `${n.id} inside the box`)
  })
}

Deno.test('two-sided: explicit side kept, the rest balanced, a pinned node off the stack', () => {
  const s = solve('mm-both')
  const n = s.layout.nodes
  assertEquals(n.get('both-1')!.grow, 'left', 'side: left')
  const sides = ['both-2', 'both-4'].map((id) => n.get(id)!.grow)
  assert(sides.includes('right'), 'auto sides balance')
  const pinned = n.get('both-3')!
  assert(pinned.pinned, 'pinned')
  const root = n.get('both-root')!
  assertEquals([pinned.x, pinned.y], [root.x + 420, root.y - 160], 'a pin is relative to the parent')
  // its children are still laid out around it
  const kid = n.get('both-3-1')!
  assert(kid.x >= pinned.x + pinned.w, 'free topic subtree grows from it')
})

Deno.test('folded subtree counts as one node', () => {
  const open = solve('mm-right')
  const folded = solve('mm-right', ['right-3'])
  snapshot('right-folded', shape(folded))
  assert(!folded.layout.nodes.has('right-3-1'), 'children of a folded node are not laid out')
  assert(folded.layout.nodes.get('right-3')!.collapsed, 'marked collapsed')
  assert(folded.layout.bounds.h < open.layout.bounds.h, 'the map gets shorter')
  // folding a leaf changes nothing
  assertEquals(shape(solve('mm-right', ['right-4'])), shape(open))
})

Deno.test('nested control and non-text node', () => {
  const s = solve('mm-min')
  snapshot('nested', shape(s))
  const inner = s.nested.get('min-inner')!
  const node = s.layout.nodes.get('min-inner')!
  assertEquals([node.w, node.h], [inner.layout.bounds.w, inner.layout.bounds.h], 'the inner box is the node size')
  assertEquals([s.layout.nodes.get('min-frame')!.w, s.layout.nodes.get('min-frame')!.h], [160, 90], 'numeric size kept')
  const placed = placeControl(s, { x: 1000, y: 500 })
  const innerRoot = placed.nodes.get('inner-root')!
  const box = placed.controls.get('min-inner')!.box
  assertEquals([box.x, box.y], [1000 + node.x, 500 + node.y], 'the inner box sits where the outer laid it')
  assert(innerRoot.x >= box.x && innerRoot.y >= box.y, 'inner nodes inside their box')
  assertEquals(innerRoot.control, 'min-inner')
})

Deno.test('slots are ordinary nodes to the layout', () => {
  snapshot('slots', shape(solve('mm-slots')))
})

Deno.test('edges: one path per branch colour and width', () => {
  const s = solve('mm-both')
  const colors = branchColors(s.layout.nodes, s.config, (id) => src.get(id)?.branch_color)
  assertEquals(colors.get('both-1'), '#e8590c', 'own branch colour')
  assertEquals(colors.get('both-1-2'), '#e8590c', 'inherited')
  assert(!colors.has('both-root'), 'a root has none')
  const strokes = edgeStrokes(s.layout.nodes, s.config, colors)
  snapshot('both-edges', strokes)
  const edges = [...s.layout.nodes.values()].filter((n) => n.parent).length
  assertEquals(strokes.reduce((n, st) => n + (st.d.match(/M/g)?.length ?? 0), 0), edges, 'every parent → child edge drawn')
})

Deno.test('a pinned width makes the title wrap and the height follow', () => {
  const long = 'A rather long node title that needs more than one line at a narrow width'
  const free = measurer.measure(long, 2)
  const narrow = measurer.measure(long, 2, 120)
  assert(narrow.lines.length > free.lines.length && narrow.h > free.h, 'wraps')
  const cjk = measurer.measure('中文标题按字符换行'.repeat(4), 2)
  assert(cjk.lines.length > 1 && cjk.w <= 260 + 28, 'CJK wraps by characters')
})

Deno.test('a thousand nodes lay out quickly', () => {
  let id = 0
  const make = (depth: number): TreeNode => ({ id: `n${id++}`, w: 80, h: 30, children: depth >= 3 ? [] : Array.from({ length: 9 }, () => make(depth + 1)) })
  const roots = [make(0)]
  const t0 = performance.now()
  const l = layoutTree(roots, mindmapConfig({ direction: 'both' }))
  const ms = performance.now() - t0
  assert(l.nodes.size > 800, `${l.nodes.size} nodes`)
  assert(ms < 50, `layout took ${ms.toFixed(1)} ms`)
})
