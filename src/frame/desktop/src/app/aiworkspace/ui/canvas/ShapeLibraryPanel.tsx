/* The shape library (draw.io 导入 TODO §2.4): the built-in libraries (draw.io stencil sets converted to SVG shapes),
 * browsed by library or searched across all of them. A click places the shape like any object; dragging it onto
 * the canvas drops it there. A library shape is an ordinary `shape` of kind `svg`: recolourable, with its own
 * connection points, and it exports back under its draw.io name. */

import { useCallback, useState } from 'react'
import { readLook } from '@drawio/shapes'
import type { Json } from '../../api/types'
import { useLoad } from '../../state/hooks'
import { ShapeArt } from '../blocks/shapeBlock'
import { SHAPE_DRAG_TYPE, stencilIndex, stencilPack, type StencilPack, type StencilPackShape } from './stencilLibrary'

function Preview({ shape }: { shape: StencilPackShape }) {
  const look = readLook({ shape: 'svg', svg: shape.svg as unknown as Json, fill: 'auto', stroke: 'auto', stroke_width: 1 })
  return <span className="aiws-library-art"><ShapeArt look={look} w={40} h={40} /></span>
}

export function ShapeLibraryPanel({ onPick }: { onPick: (shape: StencilPackShape) => void }) {
  const index = useLoad(useCallback(() => stencilIndex(), [])).data
  const [lib, setLib] = useState<string>('flowchart')
  const [query, setQuery] = useState('')
  const q = query.trim().toLowerCase()
  const loadPacks = useCallback(async (): Promise<StencilPack[]> => {
    if (!index) return []
    const libs = q ? index.libs.map((l) => l.lib) : [lib]
    return (await Promise.all(libs.map((l) => stencilPack(l)))).filter((p): p is StencilPack => p !== null)
  }, [index, lib, q])
  const packs = useLoad(loadPacks).data ?? []
  const shapes = packs.flatMap((p) => p.shapes.filter((s) => !q || s.title.toLowerCase().includes(q) || s.name.includes(q)).map((s) => ({ lib: p.lib, shape: s })))
  if (index === null) return <div className="aiws-muted">图形库没有加载（离线或部署中缺少 aiworkspace/stencils）。</div>
  return (
    <div className="aiws-library" data-testid="aiws-shape-library">
      <div className="aiws-library-head">
        <select aria-label="图形库" value={lib} data-testid="aiws-library-select" onChange={(e) => { setLib(e.target.value); setQuery('') }}>
          {(index?.libs ?? []).map((l) => <option key={l.lib} value={l.lib}>{l.title}（{l.count}）</option>)}
        </select>
        <input type="search" placeholder="搜索全部图形库…" aria-label="搜索图形" value={query} data-testid="aiws-library-search" onChange={(e) => setQuery(e.target.value)} />
      </div>
      <div className="aiws-library-grid" role="listbox" aria-label="图形">
        {shapes.slice(0, 240).map(({ lib: l, shape }) => (
          <button key={shape.name} type="button" role="option" aria-selected="false" className="aiws-library-item" title={`${shape.title}（${l}）：点击后在画布上放置，或拖到画布上`} data-testid={`aiws-library-shape-${shape.name}`}
            draggable onDragStart={(event) => { event.dataTransfer.setData(SHAPE_DRAG_TYPE, JSON.stringify({ lib: l, name: shape.name })); event.dataTransfer.effectAllowed = 'copy' }}
            onClick={() => onPick(shape)}>
            <Preview shape={shape} />
            <span className="aiws-library-name">{shape.title}</span>
          </button>
        ))}
        {shapes.length === 0 && <div className="aiws-muted">{index ? '没有匹配的图形。' : '正在加载…'}</div>}
      </div>
      <div className="aiws-muted aiws-library-note">图形来自 draw.io（jgraph/drawio，Apache-2.0）的模板库，转换为可改色的 SVG 形状。</div>
    </div>
  )
}
