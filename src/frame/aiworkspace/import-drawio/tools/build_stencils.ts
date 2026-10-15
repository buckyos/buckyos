/* Build the canvas's stencil packs from draw.io's bundled stencil sets (draw.io 导入 TODO §2.4).
 *
 *   deno run -A tools/build_stencils.ts [--source <path-or-url of stencils.min.js>] [--out <dir>]
 *                                       [--libs flowchart,basic,arrows,bpmn,networks] [--license <path-or-url>]
 *
 * The source is draw.io's `js/stencils.min.js` (by default from `dev`, draw.io's default branch on GitHub). It holds
 * every stencil file as `f['<file>.xml'] = '<base64>';`, each a raw-deflated binary encoding of the XML (Xml2Js.java
 * on draw.io's side, `decode` at the end of the bundle): five streams behind five big-endian u32 lengths — element
 * ops, three zig-zag varint number streams (x, y, others) and a NUL-separated string table. A drawing op's x / y
 * attributes are deltas of the previous ones in the same shape, every other number a delta of the same element's
 * same attribute; numbers are thousandths.
 *
 * Every `<shape>` of the chosen libraries goes through `stencilToSvg`; each library becomes `<lib>.json`
 * (`{ lib, title, package, shapes: [{ name, title, w, h, svg, anchors }] }`, sorted by name), and `index.json` maps
 * every registered name (`mxgraph.flowchart.decision`) to its library, so the canvas loads the one pack a
 * `shape=mxgraph.x.y` style needs. `LICENSE.txt` carries draw.io's Apache-2.0 notice and its stencil notice. */

import { stencilName, stencilToSvg, type StencilSvg } from '../stencil.ts'
import { base64ToBytes, inflateRaw } from '../bytes.ts'
import { elements, type XmlElement } from '../xml.ts'

export const DEFAULT_SOURCE = 'https://raw.githubusercontent.com/jgraph/drawio/dev/src/main/webapp/js/stencils.min.js'
export const DEFAULT_LICENSE = 'https://raw.githubusercontent.com/jgraph/drawio/dev/LICENSE'
export const DEFAULT_LIBS = ['flowchart', 'basic', 'arrows', 'bpmn', 'networks']

const LIB_TITLES: Record<string, string> = { flowchart: '流程图', basic: '基本图形', arrows: '箭头', bpmn: 'BPMN', networks: '网络' }

/** `f['<file>'] = '<base64>';` entries of a stencils.min.js. */
export function readStencilBundle(js: string): Map<string, string> {
  const out = new Map<string, string>()
  for (const m of js.matchAll(/^f\['([^']+)'\]\s*=\s*'([A-Za-z0-9+/=]*)';?\s*$/gm)) out.set(m[1], m[2])
  return out
}

const DRAW = new Set(['move', 'line', 'quad', 'curve', 'arc', 'rect', 'roundrect', 'ellipse', 'text', 'image', 'include-shape'])
const XS = new Set(['x', 'x1', 'x2', 'x3'])
const YS = new Set(['y', 'y1', 'y2', 'y3'])

/** One bundle entry → its root element (`<shapes name="mxgraph.…">`). A port of the bundle's `decode`. */
export async function decodeStencilFile(base64: string): Promise<XmlElement> {
  const d = await inflateRaw(base64ToBytes(base64))
  const view = new DataView(d.buffer, d.byteOffset, d.byteLength)
  const s: Uint8Array[] = []
  for (let i = 0, o = 20; i < 5; i++) {
    const n = view.getUint32(4 * i)
    s.push(d.subarray(o, o + n))
    o += n
  }
  const ops = s[0]
  const strs = new TextDecoder().decode(s[4]).split('\0')
  const p = [0, 0, 0, 0]
  let si = 0
  const names: string[] = []
  const attrNames: string[] = []
  const root: XmlElement = { name: '#root', attrs: {}, children: [] }
  const stack: XmlElement[] = [root]
  let px = 0
  let py = 0
  let pn = new Map<string, number>()
  // the next zig-zag varint of number stream k
  const read = (k: number): number => {
    const bytes = s[k]
    let n = 0
    let m = 1
    let c: number
    do {
      c = bytes[p[k]++]
      n += (c & 127) * m
      m *= 128
    } while (c & 128)
    return n % 2 === 1 ? -(n + 1) / 2 : n / 2
  }
  const format = (n: number) => String(n / 1000)
  while (p[0] < ops.length) {
    const op = ops[p[0]++]
    if (op === 255) {
      if (stack.length > 1) stack.pop()
      continue
    }
    if (op >> 1 === names.length) names.push(strs[si++])
    const name = names[op >> 1]
    const node: XmlElement = { name, attrs: Object.create(null), children: [] }
    stack[stack.length - 1].children.push(node)
    const count = ops[p[0]++]
    const coords = DRAW.has(name)
    if (name === 'shape') {
      px = 0
      py = 0
      pn = new Map()
    }
    for (let i = 0; i < count; i++) {
      const id = ops[p[0]++]
      if (id === attrNames.length) attrNames.push(strs[si++])
      const attr = attrNames[id]
      if (ops[p[0]++] === 0) node.attrs[attr] = strs[si++]
      else if (coords && XS.has(attr)) node.attrs[attr] = format(px += read(1))
      else if (coords && YS.has(attr)) node.attrs[attr] = format(py += read(2))
      else {
        const key = `${name} ${attr}`
        const next = (pn.get(key) ?? 0) + read(3)
        pn.set(key, next)
        node.attrs[attr] = format(next)
      }
    }
    // an even op opens an element, its children follow until 255
    if (!(op & 1)) stack.push(node)
  }
  const first = root.children[0]
  if (!first || typeof first === 'string') throw new Error('empty stencil file')
  return first
}

/** The stencils a file registers, by name (mxStencilRegistry.parseStencilSet; a later one replaces an earlier one). */
export function stencilEntries(root: XmlElement): { package: string; entries: Map<string, XmlElement> } {
  const sets = root.name === 'stencils' ? elements(root, 'shapes') : [root]
  const entries = new Map<string, XmlElement>()
  let pkg = ''
  for (const set of sets) {
    pkg ||= (set.attrs.name ?? '').toLowerCase()
    for (const shape of elements(set)) if (shape.attrs.name !== undefined) entries.set(stencilName(set.attrs.name, shape.attrs.name), shape)
  }
  return { package: pkg, entries }
}

export interface PackShape { name: string; title: string; w: number; h: number; svg: StencilSvg['svg']; anchors: StencilSvg['anchors'] }
export interface Pack { lib: string; title: string; package: string; shapes: PackShape[] }

/** One library's pack, plus what each instruction cost (instruction → shapes that skipped it). */
export function buildPack(lib: string, root: XmlElement): { pack: Pack; unsupported: Map<string, number> } {
  const { package: pkg, entries } = stencilEntries(root)
  const unsupported = new Map<string, number>()
  const resolve = (name: string) => entries.get(name) ?? entries.get(name.toLowerCase()) ?? null
  const shapes = [...entries.keys()].sort().map((name): PackShape => {
    const r = stencilToSvg(entries.get(name)!, { name, resolve })
    for (const u of r.unsupported) unsupported.set(u, (unsupported.get(u) ?? 0) + 1)
    return { name: r.name, title: r.title, w: r.w, h: r.h, svg: r.svg, anchors: r.anchors }
  })
  return { pack: { lib, title: LIB_TITLES[lib] ?? lib, package: pkg, shapes }, unsupported }
}

const STENCIL_NOTICE = `The icon sets and stencil libraries included in this software, and any derivatives thereof (including conversions to other formats, traced reproductions, substantially similar visual representations, or AI-generated images created using these icons as reference or training input), may not be used as software assets in, distributed for use with, or incorporated into Atlassian products or products distributed through the Atlassian marketplace or plugin ecosystem, without explicit written permission.

This restriction does not apply to end-user diagram output (such as exported images or documents) created using this software.
`

function licenseText(source: string, apache: string | null): string {
  return `The stencil packs in this folder are converted from the stencil libraries of draw.io
(https://github.com/jgraph/drawio, ${source}), copyright JGraph Ltd and draw.io AG.
Some icons are originally defined by third-party copyright holders (see draw.io's README).
draw.io is a registered trademark of JGraph Ltd; these packs are not affiliated with or endorsed by draw.io.

draw.io is licensed under the Apache License, Version 2.0 (the "License"); you may not use these files except in
compliance with the License. You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software distributed under the License is distributed on
an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the License for the
specific language governing permissions and limitations under the License.

Notice of draw.io's stencils (src/main/webapp/stencils/LICENSE):

${STENCIL_NOTICE}${apache ? `\n----------------------------------------------------------------------------------------------------------\n\n${apache}` : ''}`
}

async function load(where: string): Promise<string> {
  if (/^https?:\/\//.test(where)) {
    const res = await fetch(where)
    if (!res.ok) throw new Error(`${where}: HTTP ${res.status}`)
    return await res.text()
  }
  return await Deno.readTextFile(where)
}

function parseArgs(args: string[]): { source: string; out: URL; libs: string[]; license: string } {
  const opts: Record<string, string> = {}
  for (let i = 0; i < args.length; i++) {
    const m = /^--(source|out|libs|license)(?:=(.*))?$/.exec(args[i])
    if (!m) throw new Error(`unknown argument ${args[i]}`)
    opts[m[1]] = m[2] ?? args[++i] ?? ''
  }
  const out = opts.out
    ? new URL(opts.out.replace(/\/?$/, '/'), `file://${Deno.cwd()}/`)
    : new URL('../../../desktop/public/aiworkspace/stencils/', import.meta.url)
  const libs = (opts.libs ?? DEFAULT_LIBS.join(',')).split(',').map((l) => l.trim().replace(/\.xml$/, '')).filter(Boolean)
  return { source: opts.source ?? DEFAULT_SOURCE, out, libs, license: opts.license ?? DEFAULT_LICENSE }
}

async function main(): Promise<void> {
  const { source, out, libs, license } = parseArgs(Deno.args)
  const bundle = readStencilBundle(await load(source))
  if (!bundle.size) throw new Error(`${source}: no stencil files found`)
  await Deno.mkdir(out, { recursive: true })
  const encoder = new TextEncoder()
  const index: { lib: string; title: string; package: string; file: string; count: number; bytes: number }[] = []
  const names = new Map<string, string>()
  const totals = new Map<string, number>()
  let total = 0
  for (const lib of libs) {
    const data = bundle.get(`${lib}.xml`)
    if (data === undefined) throw new Error(`${lib}.xml is not in the bundle`)
    const { pack, unsupported } = buildPack(lib, await decodeStencilFile(data))
    const file = `${lib.replace(/\//g, '_')}.json`
    const bytes = encoder.encode(JSON.stringify(pack))
    await Deno.writeFile(new URL(file, out), bytes)
    total += bytes.length
    index.push({ lib, title: pack.title, package: pack.package, file, count: pack.shapes.length, bytes: bytes.length })
    for (const s of pack.shapes) names.set(s.name, lib)
    for (const [k, v] of unsupported) totals.set(k, (totals.get(k) ?? 0) + v)
    const skipped = [...unsupported].map(([k, v]) => `${k}×${v}`).join(', ')
    console.log(`${lib.padEnd(12)} ${String(pack.shapes.length).padStart(4)} shapes ${(bytes.length / 1024).toFixed(1).padStart(8)} KiB${skipped ? `  unsupported: ${skipped}` : ''}`)
  }
  const sortedNames = Object.fromEntries([...names].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)))
  const indexBytes = encoder.encode(JSON.stringify({ source, generated_at: new Date().toISOString(), libs: index, names: sortedNames }))
  await Deno.writeFile(new URL('index.json', out), indexBytes)
  let apache: string | null = null
  try {
    apache = await load(license)
  } catch (e) {
    console.warn(`warning: the Apache-2.0 text was not fetched (${(e as Error).message}); LICENSE.txt carries the notice only`)
  }
  await Deno.writeTextFile(new URL('LICENSE.txt', out), licenseText(source, apache))
  console.log(`index.json ${(indexBytes.length / 1024).toFixed(1)} KiB; packs ${(total / 1024).toFixed(1)} KiB in ${decodeURIComponent(out.pathname)}`)
  console.log(`unsupported (shapes): ${totals.size ? [...totals].map(([k, v]) => `${k}×${v}`).join(', ') : 'none'}`)
}

if (import.meta.main) await main()
