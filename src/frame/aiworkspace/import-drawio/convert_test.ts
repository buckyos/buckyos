/* Tests of the draw.io importer (draw.io 导入 TODO §3 测试): every carrier reads the same model, every fixture converts
 * to the plan recorded in fixtures/drawio/expected (`DRAWIO_REGEN=1 deno test -A` rewrites them), operations are
 * ordered and keyed as the core requires, and an export read back gives the same layout and bindings.
 * Run: `deno test -A` in this folder. */

import { buildOperations, convertPage, exportDiagram, ImportReport, readDrawio, type ExportNode, type PagePlan, type ResolvedStencil } from './mod.ts'
import { keysAfter } from './keys.ts'
import { htmlLabel, fillPlaceholders } from './html.ts'
import { parseStyle, resolveStyle } from './style.ts'
import { readLook, shapeAnchors, shapeOutline, shapePaths, SHAPE_KINDS } from './shapes.ts'
import { sanitizeSvgBody, svgShapeFromDocument } from './svg.ts'
import { decodeStencilStyle, stencilToSvg } from './stencil.ts'

const here = new URL('.', import.meta.url).pathname
const FIXTURES = `${here}../fixtures/drawio/`
const PACKS = `${here}../../desktop/public/aiworkspace/stencils/`

function assert(cond: unknown, message = 'assertion failed'): asserts cond {
  if (!cond) throw new Error(message)
}
function equal(a: unknown, b: unknown, message = '') {
  const x = JSON.stringify(a)
  const y = JSON.stringify(b)
  if (x !== y) throw new Error(`${message}\n  actual:   ${x.slice(0, 600)}\n  expected: ${y.slice(0, 600)}`)
}

const read = async (file: string) => readDrawio({ bytes: await Deno.readFile(`${FIXTURES}${file}`) })

/** The stencils the canvas serves, read from the packs in the Desktop app. */
const index = JSON.parse(await Deno.readTextFile(`${PACKS}index.json`)) as { libs: { lib: string; file: string }[]; names: Record<string, string> }
const packs = new Map<string, { shapes: { name: string; title: string; svg: ResolvedStencil['svg']; anchors: ResolvedStencil['anchors'] }[] }>()
const options = {
  stencil: async (name: string): Promise<ResolvedStencil | null> => {
    const lib = index.names[name.toLowerCase()]
    if (!lib) return null
    if (!packs.has(lib)) packs.set(lib, JSON.parse(await Deno.readTextFile(`${PACKS}${index.libs.find((l) => l.lib === lib)!.file}`)))
    const shape = packs.get(lib)!.shapes.find((s) => s.name === name.toLowerCase())
    return shape ? { title: shape.title, svg: shape.svg, anchors: shape.anchors, unsupported: [] } : null
  },
  inlineStencil: async (value: string): Promise<ResolvedStencil | null> => {
    const shape = await decodeStencilStyle(value)
    if (!shape) return null
    const s = stencilToSvg(shape)
    return { title: s.title, svg: s.svg, anchors: s.anchors, unsupported: s.unsupported }
  },
}

/** What a plan says, compactly (the golden files): items with their geometry, kinds, ends and the report. */
function summary(plan: PagePlan, report: ImportReport) {
  return {
    name: plan.name, bounds: plan.bounds, background: plan.background, pageSize: plan.pageSize,
    items: plan.items.map((i) => ({
      key: i.key, parent: i.parent, type: i.type, rect: i.rect, ...(i.rotation ? { rotation: i.rotation } : {}), ...(i.title ? { title: i.title } : {}), ...(i.locked ? { locked: true } : {}),
      ...(i.type === 'shape' ? { shape: i.config.shape, text: i.config.text, fill: i.config.fill, stroke: i.config.stroke, ...(i.config.anchors ? { anchors: (i.config.anchors as { id: string }[]).map((a) => a.id) } : {}) } : {}),
      ...(i.type === 'frame' ? { header: i.config.header, orientation: i.config.orientation ?? 'horizontal' } : {}),
      ...(i.image ? { image: i.image.url ?? i.image.mediaType } : {}),
      ...(i.line ? { start: i.line.start, end: i.line.end, route: i.line.route, controls: i.line.controls, label: i.line.label, caps: [i.config.start_cap ?? 'none', i.config.end_cap ?? 'arrow'] } : {}),
    })),
    report: report.list().map((n) => ({ kind: n.kind, count: n.count })),
  }
}

const FIXTURE_FILES = ['flowchart.drawio', 'swimlane.drawio', 'er.drawio', 'uml.drawio', 'multipage.drawio', 'stencil.drawio', 'shapes.drawio', 'library.xml']

for (const file of FIXTURE_FILES) {
  Deno.test(`fixture ${file} converts to its recorded plan`, async () => {
    const drawio = await read(file)
    const pages = drawio.pages.length ? drawio.pages : drawio.library.flatMap((e, i) => (e.model ? [{ id: `lib-${i}`, name: e.title, model: e.model }] : []))
    const out = []
    for (const page of pages) {
      const report = new ImportReport()
      out.push(summary(await convertPage(page, report, options), report))
    }
    const golden = `${FIXTURES}expected/${file}.plan.json`
    if (Deno.env.get('DRAWIO_REGEN')) {
      await Deno.mkdir(`${FIXTURES}expected`, { recursive: true })
      await Deno.writeTextFile(golden, JSON.stringify(out, null, 1) + '\n')
    }
    equal(out, JSON.parse(await Deno.readTextFile(golden)), `${file} differs from ${golden}`)
  })
}

Deno.test('every carrier of the flowchart reads the same model', async () => {
  const plain = await read('flowchart.drawio')
  for (const file of ['flowchart.drawio.png', 'flowchart.drawio.svg']) {
    const other = await read(file)
    equal(other.pages.map((p) => p.name), plain.pages.map((p) => p.name), file)
    const a = await convertPage(plain.pages[0], new ImportReport())
    const b = await convertPage(other.pages[0], new ImportReport())
    equal(b.items.map((i) => [i.key, i.rect]), a.items.map((i) => [i.key, i.rect]), file)
  }
  const pasted = await readDrawio({ text: encodeURIComponent((await Deno.readTextFile(`${FIXTURES}uml.drawio`)).match(/<mxGraphModel[\s\S]*<\/mxGraphModel>/)![0]) })
  equal(pasted.source, 'model')
  const multi = await read('multipage.drawio')
  equal(multi.pages.map((p) => p.name), ['系统总览', '部署'])
  const library = await read('library.xml')
  equal(library.library.map((e) => e.title), ['圆角卡片', '决策'])
})

Deno.test('operations: objects before lines, parents before children, valid keys, placements relative to parents', async () => {
  for (const file of ['swimlane.drawio', 'er.drawio', 'multipage.drawio']) {
    const plan = await convertPage((await read(file)).pages[0], new ImportReport(), options)
    const assets = new Map(plan.images.map((img) => [img.key, img.url ? null : { object_id: 'obj:00', file_name: img.name }]))
    const built = buildOperations(plan, { parentId: 'sf', origin: { x: 100, y: 50 }, afterKey: 'i', contentFolderId: 'sf-content', contentAfterKey: null, assets })
    const created = new Set(['sf', 'sf-content'])
    for (const op of [...built.objects, ...built.lines]) {
      assert(created.has(op.parent_id), `${file}: ${op.entity_id} created before its parent ${op.parent_id}`)
      assert(/^[0-9a-z]{1,64}$/.test(op.order_key) && !op.order_key.endsWith('0'), `${file}: bad order key ${op.order_key}`)
      for (const k of ['start', 'end'] as const) {
        const end = op.payload[k] as { entity_id?: string } | null | undefined
        if (end?.entity_id) assert(created.has(end.entity_id), `${file}: ${op.entity_id} bound before ${end.entity_id} exists`)
      }
      created.add(op.entity_id)
    }
    // siblings keep the plan's order; the first top-level object goes after the parent's last key
    const top = built.objects.filter((o) => o.parent_id === 'sf').map((o) => o.order_key)
    assert(top.every((k, i) => i === 0 || top[i - 1] < k) && top[0] > 'i', `${file}: top-level keys out of order`)
    const first = plan.items.find((i) => i.parent === null && i.type !== 'connector')!
    const op = built.objects.find((o) => o.entity_id === built.ids.get(first.key))!
    equal([op.placement!.x, op.placement!.y], [first.rect.x + 100, first.rect.y + 50], `${file}: top-level placement`)
  }
})

Deno.test('order keys for thousands of siblings stay short, sorted and valid', () => {
  for (const n of [1, 2, 35, 36, 1000, 5000]) {
    const keys = keysAfter('zz1', n)
    assert(keys.length === n)
    for (let i = 0; i < n; i++) {
      assert(keys[i] > 'zz1' && keys[i].length <= 12 && !keys[i].endsWith('0'), `bad key ${keys[i]}`)
      if (i) assert(keys[i - 1] < keys[i], `unsorted at ${i}`)
    }
  }
})

Deno.test('labels: whole-label formatting kept, partial formatting and structure reported, placeholders filled', () => {
  const bold = htmlLabel('<b>经理审批</b>')
  equal([bold.text, bold.bold, bold.partial], ['经理审批', true, false])
  const mixed = htmlLabel('<div><font color="#b85450">注意：</font>灰度</div><ul><li>先发</li><li>全量</li></ul>')
  equal([mixed.text, mixed.partial, mixed.complex], ['注意：灰度\n• 先发\n• 全量', true, true])
  equal(htmlLabel('a&nbsp;b<br>c &amp; d').text, 'a b\nc & d')
  equal(fillPlaceholders('%name% v%version% %missing% 100%%', { name: '网关', version: '2' }), '网关 v2 %missing% 100%')
})

Deno.test('styles: named styles, bare shape names and explicit keys', () => {
  const text = resolveStyle(parseStyle('text;html=1;align=center;'), false)
  equal([text.get('fillColor'), text.get('strokeColor'), text.get('align'), text.get('verticalAlign')], ['none', 'none', 'center', 'top'])
  equal(resolveStyle(parseStyle('ellipse;shape=cloud;'), false).get('shape'), 'cloud')
  equal(resolveStyle(parseStyle('hexagon;'), false).get('shape'), 'hexagon')
  equal(resolveStyle(parseStyle('endArrow=none;'), true).get('endArrow'), 'none')
  equal(parseStyle('shape=stencil(abc==);x=1').keys.get('shape'), 'stencil(abc==)')
})

Deno.test('shapes: every kind draws; anchors follow the outline, the direction and own points', () => {
  for (const { kind } of SHAPE_KINDS) {
    const paths = shapePaths(readLook({ shape: kind }), 160, 120)
    assert(paths.body.startsWith('M'), `${kind} draws nothing`)
  }
  const anchorsOf = (config: Record<string, unknown>) => shapeAnchors(config, shapeOutline(readLook(config)))
  const diamond = anchorsOf({ shape: 'diamond' })
  equal(['n', 'e', 's', 'w'].map((id) => diamond.find((a) => a.id === id)).map((a) => [a!.x, a!.y]), [[0.5, 0], [1, 0.5], [0.5, 1], [0, 0.5]])
  const ne = diamond.find((a) => a.id === 'ne')!
  assert(Math.abs(ne.x - 0.75) < 1e-3 && Math.abs(ne.y - 0.25) < 1e-3, 'diamond ne is the middle of its edge')
  // a triangle pointing up has its apex at n
  const up = anchorsOf({ shape: 'triangle', direction: 'n' }).find((a) => a.id === 'n')!
  equal([up.x, up.y], [0.5, 0])
  const right = anchorsOf({ shape: 'triangle' }).find((a) => a.id === 'e')!
  equal([right.x, right.y], [1, 0.5])
  const own = anchorsOf({ shape: 'rect', anchors: [{ id: 'port', x: 1, y: 0.25 }, { id: 'n', x: 0.4, y: 0 }] })
  equal(own.slice(0, 2).map((a) => a.id), ['port', 'n'])
  equal(own.filter((a) => a.id === 'n').length, 1)
})

Deno.test('svg: bodies are cleaned; a pasted document becomes recolourable', () => {
  const body = sanitizeSvgBody('<path d="M0 0" onclick="x()"/><script>alert(1)</script><foreignObject><div/></foreignObject><image href="https://evil/x.png"/><use href="#a"/>')
  equal(body, '<path d="M0 0"/><image/><use href="#a"/>')
  const doc = svgShapeFromDocument('<svg viewBox="0 0 10 10"><rect width="10" height="10" fill="#ff0000"/><circle r="2" fill="#ffffff"/></svg>')!
  equal([doc.fill, doc.svg.body.includes('var(--aiws-shape-fill)'), doc.svg.body.includes('#ffffff')], ['#ff0000', true, true])
  equal(svgShapeFromDocument('<div/>'), null)
})

/** A plan as the canvas would hold it, for the exporter: placements relative to the parent group. */
function nodesOf(plan: PagePlan): ExportNode[] {
  const byKey = new Map(plan.items.map((i) => [i.key, i]))
  return plan.items.map((i) => {
    const p = i.parent ? byKey.get(i.parent)!.rect : { x: 0, y: 0 }
    const line = i.line
    const box = line ? { x: Math.min(line.startPoint.x, line.endPoint.x), y: Math.min(line.startPoint.y, line.endPoint.y), w: Math.abs(line.endPoint.x - line.startPoint.x), h: Math.abs(line.endPoint.y - line.startPoint.y) } : i.rect
    return {
      id: i.key, parent: i.parent, kind: i.type === 'group' ? 'group' : 'cell', view: i.type === 'asset' ? 'asset' : i.type,
      placement: { x: box.x - p.x, y: box.y - p.y, w: box.w, h: box.h, ...(i.rotation ? { rotation: i.rotation } : {}) },
      ...(i.title ? { title: i.title } : {}), config: i.config, ...(i.locked ? { locked: true } : {}),
      ...(i.image ? { image: 'data:image/png;base64,iVBORw0KGgo=' } : {}),
      ...(line ? { line: {
        start: line.start ? { entity_id: line.start.key, anchor: { kind: 'named', id: line.start.anchor } } : null,
        end: line.end ? { entity_id: line.end.key, anchor: { kind: 'named', id: line.end.anchor } } : null,
        flip: { h: line.startPoint.x > line.endPoint.x, v: line.startPoint.y > line.endPoint.y }, route: line.route, controls: line.controls,
        ...(line.label ? { label: line.label } : {}),
      } } : {}),
    }
  })
}

for (const file of ['flowchart.drawio', 'swimlane.drawio', 'er.drawio', 'uml.drawio', 'stencil.drawio', 'shapes.drawio']) {
  Deno.test(`round trip ${file}: export, import again, same layout and bindings`, async () => {
    const plan = await convertPage((await read(file)).pages[0], new ImportReport(), options)
    const { xml } = exportDiagram(nodesOf(plan), { name: plan.name, background: plan.background, pageSize: plan.pageSize })
    const again = await convertPage((await readDrawio({ text: xml })).pages[0], new ImportReport(), options)
    const twins = new Map(again.items.map((i) => [i.key, i]))
    for (const item of plan.items) {
      if (item.key.includes('~')) continue // a container's backing is its group again
      const twin = twins.get(item.key)
      assert(twin, `${file}: ${item.key} missing after the round trip`)
      if (item.line) {
        equal([twin.line?.start, twin.line?.end, twin.line?.route], [item.line.start, item.line.end, item.line.route], `${file}: ${item.key} ends`)
        continue
      }
      for (const k of ['x', 'y', 'w', 'h'] as const) assert(Math.abs(twin.rect[k] - item.rect[k]) <= 1, `${file}: ${item.key}.${k} ${twin.rect[k]} vs ${item.rect[k]}`)
      if (item.type === 'shape') equal([twin.config.shape, twin.config.fill, twin.config.stroke, twin.config.text], [item.config.shape, item.config.fill, item.config.stroke, item.config.text], `${file}: ${item.key} look`)
    }
  })
}
