import type { Json } from '../../api/types'
import type { WorkspaceSession } from '../../api/session'

export const ROW_HEIGHT = 34
export const MIN_ROW_HEIGHT = 28
export const MAX_ROW_HEIGHT = 320

export type GridStyle = 'grid' | 'striped' | 'lines'

export function tableAppearance(config: Record<string, Json> = {}) {
  const style: GridStyle = config.grid_style === 'striped' || config.grid_style === 'lines' ? config.grid_style : 'grid'
  const heights = config.row_heights
  const rowHeights: Record<string, number> = Object.create(null)
  if (heights && typeof heights === 'object' && !Array.isArray(heights)) {
    for (const [id, px] of Object.entries(heights)) if (typeof px === 'number' && Number.isFinite(px)) rowHeights[id] = Math.max(MIN_ROW_HEIGHT, Math.min(px, MAX_ROW_HEIGHT))
  }
  return { style, showHeader: config.show_header !== false, rowHeights }
}

export async function cleanRowHeights(session: WorkspaceSession, sourceId: string, heights: Record<string, number>, visible: Set<string>) {
  const ids = Object.keys(heights).filter((id) => !visible.has(id))
  for (let i = 0; i < ids.length; i += 64) {
    const batch = ids.slice(i, i + 64)
    const result = await session.readMany(batch.map((id) => ({ entity_id: sourceId, selector: { kind: 'table_record', record_id: id } })))
    result.forEach((r, j) => { if ('error' in r && ['TARGET_DELETED', 'NOT_FOUND'].includes(r.error.code)) delete heights[batch[j]] })
  }
  return heights
}
