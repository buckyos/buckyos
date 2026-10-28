/* What a mind map draws around its nodes (脑图方案 §6.1, R4, R23: template children, not entities): the implicit
 * edges — one SVG under the nodes, a path per branch colour and width — and the pin of a free topic. (The fold
 * handles are screen-sized controls: the canvas overlay draws them.) While a node is dragged the edges follow the
 * proposed layout and a dashed line shows where it would hang. */

import { useMemo } from 'react'
import { Pin } from 'lucide-react'
import { edgePath, edgeStrokes, edgeWidth } from '@controls/edges'
import type { LaidNode } from '@controls/tree'
import { useAiwsText } from '../../../useAiwsText'
import type { ControlChromeProps } from '../registry'

export function MindmapChrome({ container, laid, preview }: ControlChromeProps) {
  const { t } = useAiwsText()
  const id = container.entity.entity_id
  const box = container.rect
  const config = container.control!.placed.solved.config
  const { nodes, colors } = useMemo(() => {
    const nodes = new Map<string, LaidNode>()
    const colors = new Map<string, string>()
    for (const l of laid.values()) {
      const n = l.node
      if (!n || n.control !== id || preview?.dragged.has(l.entity.entity_id)) continue
      const r = preview?.rects.get(l.entity.entity_id) ?? l.rect
      nodes.set(l.entity.entity_id, { id: l.entity.entity_id, x: r.x - box.x, y: r.y - box.y, w: r.w, h: r.h, parent: n.parent, depth: n.depth, grow: n.grow, pinned: n.pinned, hasChildren: n.hasChildren, collapsed: n.collapsed })
      if (n.color) colors.set(l.entity.entity_id, n.color)
    }
    return { nodes, colors }
  }, [laid, id, box.x, box.y, preview])
  const strokes = useMemo(() => edgeStrokes(nodes, config, colors), [nodes, config, colors])
  // the proposed hanging point of a dragged node
  const hang = preview?.parent && preview.at ? (() => {
    const parent = nodes.get(preview.parent)
    if (!parent) return null
    const at = { ...preview.at, x: preview.at.x - box.x, y: preview.at.y - box.y }
    return { d: edgePath(parent, at, parent.grow === 'down' ? 'down' : at.x + at.w / 2 >= parent.x + parent.w / 2 ? 'right' : 'left', config.edge), width: edgeWidth(parent.depth + 1) }
  })() : null
  return (
    <>
      <svg className="aiws-control-edges" width={Math.max(1, box.w)} height={Math.max(1, box.h)} aria-hidden="true" data-testid={`aiws-mindmap-edges-${id}`} data-edges={[...nodes.values()].filter((n) => n.parent && nodes.has(n.parent)).length}>
        {/* a map inside a node shows its box: the edge into it ends there */}
        {container.node && <rect className="aiws-control-box" x={0.5} y={0.5} width={Math.max(0, box.w - 1)} height={Math.max(0, box.h - 1)} rx={10} />}
        {strokes.map((s) => <path key={`${s.color}|${s.width}`} d={s.d} stroke={s.color} strokeWidth={s.width} fill="none" strokeLinecap="round" />)}
        {hang && <path className="aiws-control-hang" d={hang.d} strokeWidth={hang.width} fill="none" data-testid="aiws-mindmap-hang" />}
      </svg>
      {[...nodes.values()].filter((n) => n.pinned).map((n) => (
        <span key={`pin:${n.id}`} className="aiws-pin-mark" style={{ left: n.x + n.w - 7, top: n.y - 9 }} title={t('aiworkspace.mindmap.pinned')} data-testid={`aiws-pin-${n.id}`}>
          <Pin size={10} aria-hidden="true" />
        </span>
      ))}
    </>
  )
}
