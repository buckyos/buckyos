/* Derives the carrier variants of the draw.io fixtures from the plain sources in `src/` (draw.io 导入 TODO §3 测试):
 * compressed pages, an inline `stencil(…)`, an `<mxlibrary>`, a `.drawio.png` and a `.drawio.svg`. Compression is
 * draw.io's `Graph.compress`: base64(deflateRaw(encodeURIComponent(xml))). Run: `deno run -A derive.ts`. */

const here = new URL('.', import.meta.url).pathname
const read = (p: string) => Deno.readTextFileSync(here + p)

async function pipe(bytes: Uint8Array, format: 'deflate' | 'deflate-raw'): Promise<Uint8Array> {
  return new Uint8Array(await new Response(new Blob([bytes as BlobPart]).stream().pipeThrough(new CompressionStream(format))).arrayBuffer())
}
function b64(bytes: Uint8Array): string { let s = ''; for (const b of bytes) s += String.fromCharCode(b); return btoa(s) }
async function compress(xml: string): Promise<string> { return b64(await pipe(new TextEncoder().encode(encodeURIComponent(xml)), 'deflate-raw')) }

// pages compressed
let multipage = read('src/multipage.drawio')
for (const model of multipage.match(/<mxGraphModel[\s\S]*?<\/mxGraphModel>/g) ?? []) multipage = multipage.replace(model, await compress(model))
Deno.writeTextFileSync(here + 'multipage.drawio', multipage)

// inline stencil and library entries
let stencil = read('src/stencil.drawio')
for (const [all, file] of stencil.matchAll(/@@([\w.-]+)@@/g)) stencil = stencil.replace(all, await compress(read(`src/${file}`).trim()))
Deno.writeTextFileSync(here + 'stencil.drawio', stencil)
let library = read('src/library.xml')
for (const [all, file] of library.matchAll(/@@([\w.-]+)@@/g)) library = library.replace(all, await compress(read(`src/${file}.xml`).trim()))
Deno.writeTextFileSync(here + 'library.xml', library)

// .drawio.png: a small picture with the file in a tEXt chunk (URL-encoded, as draw.io writes it)
const CRC = new Uint32Array(256).map((_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0 })
const crc32 = (bytes: Uint8Array) => { let c = 0xffffffff; for (const b of bytes) c = CRC[(c ^ b) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0 }
function chunk(type: string, data: Uint8Array): Uint8Array {
  const out = new Uint8Array(12 + data.length)
  const view = new DataView(out.buffer)
  view.setUint32(0, data.length)
  out.set(new TextEncoder().encode(type), 4)
  out.set(data, 8)
  view.setUint32(8 + data.length, crc32(out.subarray(4, 8 + data.length)))
  return out
}
const flowchart = read('flowchart.drawio')
const w = 120, h = 80
const raw = new Uint8Array((w * 3 + 1) * h)
for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) { const i = y * (w * 3 + 1) + 1 + x * 3; const inside = x > 20 && x < 100 && y > 20 && y < 60; raw[i] = inside ? 218 : 255; raw[i + 1] = inside ? 232 : 255; raw[i + 2] = inside ? 252 : 255 }
const ihdr = new Uint8Array(13)
new DataView(ihdr.buffer).setUint32(0, w); new DataView(ihdr.buffer).setUint32(4, h); ihdr.set([8, 2, 0, 0, 0], 8)
const text = new TextEncoder().encode(`mxfile\u0000${encodeURIComponent(flowchart)}`)
const parts = [new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('tEXt', text), chunk('IDAT', await pipe(raw, 'deflate')), chunk('IEND', new Uint8Array())]
const png = new Uint8Array(parts.reduce((n, p) => n + p.length, 0))
let at = 0
for (const p of parts) { png.set(p, at); at += p.length }
Deno.writeFileSync(here + 'flowchart.drawio.png', png)

// .drawio.svg: the file in the root's `content` attribute
const attr = flowchart.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/\n/g, '&#10;')
Deno.writeTextFileSync(here + 'flowchart.drawio.svg', `<svg xmlns="http://www.w3.org/2000/svg" width="571" height="581" viewBox="-0.5 -0.5 571 581" content="${attr}"><rect x="10" y="0" width="120" height="60" fill="#d5e8d4" stroke="#82b366"/></svg>\n`)
console.log('derived: multipage.drawio, stencil.drawio, library.xml, flowchart.drawio.png, flowchart.drawio.svg')
