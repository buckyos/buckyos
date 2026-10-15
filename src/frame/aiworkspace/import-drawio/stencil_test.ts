/* Tests for stencil.ts (mxStencil → SVG body) and the bundle decoder of tools/build_stencils.ts.
 * Run: deno test -A stencil_test.ts */

import { bytesToBase64, deflateRaw } from './bytes.ts'
import { decodeStencilStyle, stencilFile, stencilName, stencilToSvg } from './stencil.ts'
import { decodeStencilFile, readStencilBundle, stencilEntries } from './tools/build_stencils.ts'
import { parseXml } from './xml.ts'

function assert(condition: unknown, message = 'assertion failed'): asserts condition {
  if (!condition) throw new Error(message)
}
function assertEquals(actual: unknown, expected: unknown, message = ''): void {
  const a = JSON.stringify(actual)
  const e = JSON.stringify(expected)
  if (a !== e) throw new Error(`${message ? message + ': ' : ''}expected ${e}, got ${a}`)
}
function assertStringIncludes(actual: string, expected: string): void {
  if (!actual.includes(expected)) throw new Error(`expected ${JSON.stringify(actual)} to include ${JSON.stringify(expected)}`)
}

const svgOf = (xml: string, resolve?: (name: string) => ReturnType<typeof parseXml> | null) => stencilToSvg(parseXml(xml), { name: 'test.shape', resolve })

/** The SVG elements of a body (inside the defaults group), as [tag, attribute text]. */
const items = (body: string) => [...body.matchAll(/<(path|rect|ellipse|text)\b([^>]*)>/g)].map((m) => [m[1], m[2]] as const)

Deno.test('background fillstroke, foreground strokes, fixed colours and connections', () => {
  const r = svgOf(`
    <shape name="Test Shape" w="100" h="50" aspect="variable" strokewidth="inherit">
      <connections>
        <constraint x="0.5" y="0" perimeter="0" name="N"/>
        <constraint x="1" y="0.5" perimeter="0" name="E side"/>
        <constraint x="0" y="0.5" perimeter="0"/>
        <constraint x="0.5" y="1" perimeter="0" name="N"/>
        <constraint x="0.25" y="0.5" perimeter="1" name="P"/>
      </connections>
      <background>
        <path><move x="0" y="0"/><line x="100" y="0"/><line x="100" y="50"/><close/></path>
      </background>
      <foreground>
        <fillstroke/>
        <strokecolor color="#FF0000"/>
        <strokewidth width="2"/>
        <path><move x="0" y="25"/><line x="100" y="25"/></path>
        <stroke/>
        <fillcolor color="none"/>
        <ellipse x="10" y="10" w="20" h="20"/>
        <fillstroke/>
        <fillcolor color="stroke"/>
        <rect x="40" y="10" w="10" h="10"/>
        <fill/>
      </foreground>
    </shape>`)
  assertEquals(r.name, 'test.shape')
  assertEquals(r.title, 'Test Shape')
  assertEquals([r.w, r.h], [100, 50])
  assertEquals(r.svg.viewBox, '0 0 100 50')
  assertEquals(r.svg.aspect, 'stretch')
  assertStringIncludes(r.svg.body, 'fill:var(--aiws-shape-fill);stroke:var(--aiws-shape-stroke);stroke-width:var(--aiws-shape-stroke-width)')
  const els = items(r.svg.body)
  assertEquals(els.map(([tag]) => tag), ['path', 'path', 'ellipse', 'rect'])
  // the background outline, filled and stroked with the shape's own colours
  assertStringIncludes(els[0][1], 'd="M0 0L100 0L100 50Z"')
  assertEquals(els[0][1].includes('fill:'), false)
  assertStringIncludes(els[0][1], 'vector-effect:non-scaling-stroke')
  // a foreground stroke over it: fixed colour, twice the shape's width, never filled
  assertStringIncludes(els[1][1], 'd="M0 25L100 25"')
  assertStringIncludes(els[1][1], 'fill:none')
  assertStringIncludes(els[1][1], 'stroke:#ff0000')
  assertStringIncludes(els[1][1], 'stroke-width:calc(var(--aiws-shape-stroke-width) * 2)')
  assertStringIncludes(els[2][1], 'cx="20" cy="20" rx="10" ry="10"')
  assertStringIncludes(els[2][1], 'fill:none;stroke:#ff0000')
  // fill only, in the shape's stroke colour
  assertStringIncludes(els[3][1], 'fill:var(--aiws-shape-stroke);stroke:none')
  for (const [, a] of els) if (!a.includes('stroke:none')) assertStringIncludes(a, 'vector-effect:non-scaling-stroke')
  assertEquals(r.anchors, [
    { id: 'N', x: 0.5, y: 0 }, { id: 'E_side', x: 1, y: 0.5 }, { id: 'c2', x: 0, y: 0.5 }, { id: 'N_2', x: 0.5, y: 1 },
    { id: 'P', x: 0, y: 0.5 },
  ])
  assertEquals(r.unsupported, [])
})

Deno.test('arc becomes an SVG arc; quad and curve keep their control points', () => {
  const r = svgOf(`
    <shape name="Arc" w="100" h="50"><foreground>
      <path><move x="0" y="50"/><arc rx="50" ry="50" x-axis-rotation="0" large-arc-flag="0" sweep-flag="1" x="100" y="50"/><close/></path>
      <fillstroke/>
      <path><move x="0" y="0"/><quad x1="10" y1="20" x2="30" y2="0"/><curve x1="40" y1="10" x2="50.123" y2="-10" x3="60" y3="0"/></path>
      <stroke/>
    </foreground></shape>`)
  const els = items(r.svg.body)
  assertStringIncludes(els[0][1], 'd="M0 50A50 50 0 0 1 100 50Z"')
  assertStringIncludes(els[1][1], 'd="M0 0Q10 20 30 0C40 10 50.12 -10 60 0"')
})

Deno.test('roundrect takes arcsize as a percentage of the shorter side (15 by default)', () => {
  const r = svgOf(`
    <shape name="Round" w="100" h="40"><foreground>
      <roundrect x="0" y="0" w="100" h="40" arcsize="25"/><fillstroke/>
      <roundrect x="0" y="0" w="100" h="40"/><stroke/>
    </foreground></shape>`)
  const els = items(r.svg.body)
  assertStringIncludes(els[0][1], 'width="100" height="40" rx="10" ry="10"')
  assertStringIncludes(els[1][1], 'rx="6" ry="6"')
})

Deno.test('save and restore keep the state stack', () => {
  const r = svgOf(`
    <shape name="State" w="100" h="100"><foreground>
      <save/>
      <fillcolor color="#00ff00"/><strokewidth width="3" fixed="1"/><dashed dashed="1"/><alpha alpha="0.5"/>
      <rect x="0" y="0" w="50" h="50"/><fillstroke/>
      <restore/>
      <rect x="50" y="50" w="50" h="50"/><fillstroke/>
      <restore/>
      <dashed dashed="1"/><dashpattern pattern="4 2"/><strokewidth width="2"/>
      <path><move x="0" y="100"/><line x="100" y="0"/></path><stroke/>
    </foreground></shape>`)
  const [first, second, third] = items(r.svg.body)
  assertStringIncludes(first[1], 'fill:#00ff00')
  assertStringIncludes(first[1], 'stroke-width:3')
  assertStringIncludes(first[1], 'stroke-dasharray:9 9') // the default 3 3, in stroke widths
  assertStringIncludes(first[1], 'fill-opacity:0.5')
  assertStringIncludes(first[1], 'stroke-opacity:0.5')
  assertEquals(second[1].includes('fill:'), false)
  assertEquals(second[1].includes('stroke-width'), false)
  assertEquals(second[1].includes('dasharray'), false)
  assertEquals(second[1].includes('opacity'), false)
  assertStringIncludes(third[1], 'stroke-dasharray:calc(var(--aiws-shape-stroke-width) * 8) calc(var(--aiws-shape-stroke-width) * 4)')
})

Deno.test('shape attributes: fixed aspect, numeric stroke width, default size', () => {
  const r = svgOf(`<shape name="Fixed" w="40" h="20" aspect="fixed" strokewidth="2"><foreground><rect x="0" y="0" w="40" h="20"/><stroke/></foreground></shape>`)
  assertEquals(r.svg.aspect, 'fixed')
  assertStringIncludes(r.svg.body, '<g style="fill:var(--aiws-shape-fill);stroke:var(--aiws-shape-stroke);stroke-width:calc(var(--aiws-shape-stroke-width) * 2)')
  const d = svgOf(`<shape name="Default"><foreground><ellipse x="0" y="0" w="100" h="100"/><fill/></foreground></shape>`)
  assertEquals([d.w, d.h, d.svg.viewBox], [100, 100, '0 0 100 100'])
})

Deno.test('colours: shape colours, style keys with defaults, rejected values', () => {
  const r = svgOf(`
    <shape name="Colours" w="10" h="10"><foreground>
      <fillcolor color="fillColor2" default="#ABCDEF"/><rect x="0" y="0" w="1" h="1"/><fill/>
      <fillcolor color="strokeColor"/><rect x="1" y="0" w="1" h="1"/><fill/>
      <fillcolor color="red;background:url(x)"/><rect x="2" y="0" w="1" h="1"/><fillstroke/>
      <fillcolor color="rgb(1, 2, 3)"/><strokecolor color="none"/><rect x="3" y="0" w="1" h="1"/><fillstroke/>
    </foreground></shape>`)
  const els = items(r.svg.body)
  assertStringIncludes(els[0][1], 'fill:#abcdef')
  assertStringIncludes(els[1][1], 'fill:var(--aiws-shape-stroke)')
  assertStringIncludes(els[2][1], 'fill:none') // not a colour, no default: no fill
  assertEquals(r.svg.body.includes('url('), false)
  assertStringIncludes(els[3][1], 'fill:rgb(1, 2, 3);stroke:none')
})

Deno.test('text: alignment, size, style, colour and escaping', () => {
  const r = svgOf(`
    <shape name="Text" w="100" h="100"><foreground>
      <fontsize size="20"/><fontstyle style="3"/><fontfamily family="Helvetica;x:url(y)"/>
      <text str="A&lt;script&gt;" x="50" y="50" align="center" valign="middle"/>
      <fontcolor color="#123456"/>
      <text str="one&#10;two" x="0" y="0" vertical="1"/>
    </foreground></shape>`)
  const texts = [...r.svg.body.matchAll(/<text\b[^>]*>.*?<\/text>/g)].map((m) => m[0])
  assertEquals(texts.length, 2)
  assertStringIncludes(texts[0], 'x="50" y="59"') // baseline: 50 + 20 - 1 - 20 / 2
  assertStringIncludes(texts[0], 'fill:var(--aiws-shape-font, currentColor);stroke:none;font-size:20px;font-family:Helveticaxurly')
  assertStringIncludes(texts[0], 'font-weight:bold;font-style:italic;text-anchor:middle')
  assertStringIncludes(texts[0], '>A&lt;script&gt;</text>')
  assertStringIncludes(texts[1], 'fill:#123456')
  assertStringIncludes(texts[1], 'transform="rotate(-90 0 0)"')
  assertStringIncludes(texts[1], '<tspan x="0" dy="0">one</tspan><tspan x="0" dy="24">two</tspan>')
  assertEquals(/<script|\son\w+=|foreignObject/i.test(r.svg.body), false)
})

Deno.test('unsupported instructions are listed; include-shape draws through the resolver', () => {
  const r = svgOf(`
    <shape name="Mixed" w="100" h="100"><foreground>
      <image src="http://example.com/a.png" x="0" y="0" w="10" h="10"/>
      <include-shape name="mxgraph.basic.inner" x="0" y="0" w="50" h="50"/>
      <frobnicate/>
      <rect x="0" y="0" w="100" h="100"/><stroke/>
    </foreground></shape>`)
  assertEquals(r.unsupported.sort(), ['frobnicate', 'image', 'include-shape'])
  assertEquals(r.svg.body.includes('example.com'), false)
  const inner = parseXml(`<shape name="Inner" w="10" h="20" strokewidth="inherit"><foreground><ellipse x="0" y="0" w="10" h="20"/><fillstroke/></foreground></shape>`)
  const resolved = svgOf(`
    <shape name="Outer" w="100" h="100"><foreground>
      <include-shape name="mxgraph.basic.inner" x="0" y="0" w="50" h="50"/>
    </foreground></shape>`, (name) => (name === 'mxgraph.basic.inner' ? inner : null))
  assertEquals(resolved.unsupported, [])
  assertStringIncludes(resolved.svg.body, '<ellipse cx="25" cy="25" rx="25" ry="25"')
})

Deno.test('a stencil that draws nothing becomes a titled rectangle', () => {
  const r = svgOf(`<shape name="Picture" w="80" h="40"><foreground><image src="data:image/png;base64,AAAA" x="0" y="0" w="80" h="40"/></foreground></shape>`)
  assertEquals(r.unsupported.sort(), ['empty', 'image'])
  assertStringIncludes(r.svg.body, '<rect x="0" y="0" width="80" height="40" style="vector-effect:non-scaling-stroke"/>')
  assertStringIncludes(r.svg.body, '>Picture</text>')
  assertEquals(r.svg.body.includes('data:'), false)
})

Deno.test('rounded paths round their corners', () => {
  const r = svgOf(`<shape name="R" w="100" h="100"><foreground>
    <path rounded="1" arcSize="10"><move x="0" y="0"/><line x="100" y="0"/><line x="100" y="100"/><line x="0" y="0"/></path><stroke/>
  </foreground></shape>`)
  const d = /d="([^"]+)"/.exec(r.svg.body)![1]
  assert(d.startsWith('M') && d.includes('Q100 0 ') && d.endsWith('Z'), d)
})

Deno.test('names and files follow mxStencilRegistry', () => {
  assertEquals(stencilName('mxGraph.flowchart', 'Or Gate'), 'mxgraph.flowchart.or_gate')
  assertEquals(stencilName(undefined, 'Box'), 'box')
  assertEquals(stencilFile('mxgraph.flowchart.decision'), 'flowchart.xml')
  assertEquals(stencilFile('mxgraph.cisco.routers.router'), 'cisco/routers.xml')
  assertEquals(stencilFile('mxgraph.flowchart'), 'flowchart.xml')
  assertEquals(stencilFile('rectangle'), null)
})

/** draw.io's Graph.compress: base64 of the raw deflate of the URL-encoded text. */
async function compress(text: string, urlEncode = true): Promise<string> {
  return bytesToBase64(await deflateRaw(new TextEncoder().encode(urlEncode ? encodeURIComponent(text) : text)))
}

Deno.test('decodeStencilStyle reads shape=stencil(...) values', async () => {
  const xml = '<shape name="Ünïcode 50%" w="20" h="10" aspect="variable" strokewidth="inherit"><background><rect x="0" y="0" w="20" h="10"/></background><foreground><fillstroke/></foreground></shape>'
  const value = await compress(xml)
  for (const v of [value, `stencil(${value})`, await compress(xml, false)]) {
    const shape = await decodeStencilStyle(v)
    assert(shape)
    assertEquals(shape.name, 'shape')
    assertEquals(shape.attrs.name, 'Ünïcode 50%')
    assertStringIncludes(stencilToSvg(shape).svg.body, '<rect x="0" y="0" width="20" height="10"')
  }
  // draw.io's default custom shape (EditorUi.defaultCustomShapeStyle)
  const custom = await decodeStencilStyle('tZRtTsQgEEBPw1+DJR7AoN6DbWftpAgE0Ortd/jYRGq72R+YNE2YgTePloEJGWblgA18ZuKFDcMj5/Sm8boZq+BgjCX4pTyqk6ZlKROitwusOMXKQDODx5iy4pXxZ5qTHiFHawxB0JrQZH7lCabQ0Fr+XWC1/E8zcsT/gAi+Subo2/3Mh6d/oJb5nU1b5tW7r2knautaa3T+U32o7f7vZwpJkaNDLORJjcu7t59m2jXxqX9un+tt022acsfmoKaQZ+vhhswZtS6Ne/ThQGt0IV0N3Yyv6P3CeT9/tHO0XFI5cAE=')
  assert(custom)
  const r = stencilToSvg(custom)
  assertEquals(items(r.svg.body).map(([tag]) => tag), ['rect', 'ellipse'])
  assertEquals(r.anchors.length, 8)
  assertEquals(await decodeStencilStyle('not base64 !'), null)
  assertEquals(await decodeStencilStyle(await compress('<mxGraphModel/>')), null)
})

const BUNDLE = '/tmp/dev-cache-root/tmp/drawio-src/stencils.min.js'
const bundleText = (() => {
  try {
    return Deno.readTextFileSync(BUNDLE)
  } catch {
    return null
  }
})()

Deno.test({ name: 'the real stencils.min.js decodes to the registered shapes', ignore: bundleText === null, fn: async () => {
  const bundle = readStencilBundle(bundleText!)
  assert(bundle.size > 100)
  const root = await decodeStencilFile(bundle.get('flowchart.xml')!)
  assertEquals(root.name, 'shapes')
  const { package: pkg, entries } = stencilEntries(root)
  assertEquals(pkg, 'mxgraph.flowchart')
  const decision = entries.get('mxgraph.flowchart.decision')
  assert(decision)
  const r = stencilToSvg(decision, { name: 'mxgraph.flowchart.decision' })
  assertEquals([r.title, r.w, r.h], ['Decision', 100, 100])
  assertStringIncludes(r.svg.body, '<path d="M50 0L100 50L50 100L0 50Z" style="vector-effect:non-scaling-stroke"/>')
  assertEquals(r.anchors.map((a) => a.id), ['N', 'S', 'W', 'E'])
  // a library with arcs, roundrects and save / restore converts without skipping anything
  const bpmn = stencilEntries(await decodeStencilFile(bundle.get('bpmn.xml')!))
  for (const [name, shape] of bpmn.entries) assertEquals(stencilToSvg(shape, { name }).unsupported, [], name)
} })
