/* The built-in shape libraries (draw.io 导入 TODO §2.4, Q6): draw.io's XML stencil sets, converted to SVG shape
 * bodies at build time (src/frame/aiworkspace/import-drawio/tools/build_stencils.ts) and served as one pack per
 * library from `public/aiworkspace/stencils/`. Packs load on demand: the library panel and the importer (a
 * `shape=mxgraph.<lib>.<name>` style) read them through here; the index maps a shape name to its pack. */

import type { ResolvedStencil } from '@drawio/convert'
import type { SvgBody } from '@drawio/shapes'
import type { Json } from '../../api/types'

export interface StencilPackShape { name: string; title: string; w: number; h: number; svg: SvgBody; anchors: { id: string; x: number; y: number }[] }
export interface StencilPack { lib: string; title: string; package: string; shapes: StencilPackShape[] }
export interface StencilIndex { libs: { lib: string; title: string; package: string; file: string; count: number; bytes: number }[]; names: Record<string, string> }

const BASE = `${import.meta.env.BASE_URL}aiworkspace/stencils/`

let index: Promise<StencilIndex | null> | null = null
const packs = new Map<string, Promise<StencilPack | null>>()

async function getJson<T>(file: string): Promise<T | null> {
  try {
    const response = await fetch(`${BASE}${file}`)
    return response.ok ? ((await response.json()) as T) : null
  } catch {
    return null
  }
}

export function stencilIndex(): Promise<StencilIndex | null> {
  if (!index) index = getJson<StencilIndex>('index.json').then((value) => { if (!value) index = null; return value })
  return index
}

export async function stencilPack(lib: string): Promise<StencilPack | null> {
  let pack = packs.get(lib)
  if (!pack) {
    const file = (await stencilIndex())?.libs.find((l) => l.lib === lib)?.file
    if (!file) return null
    pack = getJson<StencilPack>(file).then((value) => { if (!value) packs.delete(lib); return value })
    packs.set(lib, pack)
  }
  return pack
}

/** A library shape by its style name (`mxgraph.flowchart.decision`; draw.io matches it lower-cased). */
export async function libraryShape(name: string): Promise<ResolvedStencil | null> {
  const key = name.toLowerCase()
  const lib = (await stencilIndex())?.names[key]
  if (!lib) return null
  const shape = (await stencilPack(lib))?.shapes.find((s) => s.name === key)
  return shape ? { title: shape.title, svg: shape.svg, anchors: shape.anchors, unsupported: [] } : null
}

/** What dragging a library shape carries (the canvas drop reads it). */
export const SHAPE_DRAG_TYPE = 'application/x-aiws-library-shape'

/** The insertion of a library shape: its size (at most 120 on the longer side) and configuration. */
export function libraryShapeConfig(shape: StencilPackShape): { size: { w: number; h: number }; config: Record<string, Json> } {
  const k = Math.min(1, 120 / Math.max(shape.w, shape.h, 1))
  return {
    size: { w: Math.max(16, Math.round(shape.w * k)), h: Math.max(16, Math.round(shape.h * k)) },
    config: {
      shape: 'svg', svg: shape.svg as unknown as Json, fill: 'auto', stroke: 'auto', stroke_width: 1,
      ...(shape.anchors.length ? { anchors: shape.anchors as unknown as Json } : {}),
      drawio: { style: `shape=${shape.name};html=1;` },
    },
  }
}
