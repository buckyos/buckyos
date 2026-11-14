/* TableView cell (design §3.5; UI 框架 PRD §7.3): a table-view Block's view of a TableSource, or the table itself, in
 * the standard table editor (ui/table/TableEditor.tsx). Every write carries the rev the user was looking at when the
 * edit began. */

import { useCallback } from 'react'
import type { CapturedAnchor } from '../anchors/registry'
import type { CellPayload, KeyedContent } from '../api/types'
import { useLoad, useStore, useVersion, type AnnotationMark } from '../state/hooks'
import { useAiwsText } from '../useAiwsText'
import type { TableEditorSize } from './table/model'
import { TableEditorBody, type CellRead } from './table/TableEditor'

export { ConflictBox } from './table/ConflictBox'

interface Props {
  /** A table-view Cell; or, without one, the table itself (data-source view, phase two §6.1). */
  cellId?: string
  sourceId?: string
  /** No editing at all (embedded rendering, missing capability, write lock not held). */
  readOnly: boolean
  /** Why nothing can be edited, for the title row (else worked out from the capability, the source and the connection). */
  readOnlyReason?: string | null
  /** The static thumbnail of a Block: the grid only, no tools. */
  compact?: boolean
  /** Where the editor sits (§7.1); a bounded box by default. */
  size?: TableEditorSize
  annotations: AnnotationMark[]
  onAnnotate?: (anchor: CapturedAnchor) => void
  onActivateAnnotation?: (entityId: string | null) => void
}

export function TableViewCell({ cellId, sourceId, readOnly, readOnlyReason, compact, size = 'embedded', annotations, onAnnotate, onActivateAnnotation }: Props) {
  const { t } = useAiwsText()
  const store = useStore()
  const id = cellId ?? ''
  const cellVersion = useVersion(`e:${id}`)
  const loadCell = useCallback(() => (cellId ? store.session.read<KeyedContent<CellPayload>>(cellId) : Promise.resolve(null)), [store, cellId])
  const cell = useLoad<CellRead | null>(loadCell, cellVersion)
  const common = { readOnly, readOnlyReason, compact, size, annotations, onAnnotate, onActivateAnnotation }
  if (!cellId && sourceId) {
    // no Block: the table itself, with the sort / filter and the columns kept in the user work state (§4.4)
    const synthetic: CellRead = {
      entity_id: `source:${sourceId}`, type_id: 'buckyos.cell', schema_version: 1, name: null, scope: 'shared', deleted: false, content_rev: 0, meta_rev: 0, life_rev: 0,
      write_policy: 'open', capabilities: [], head_seq: 0,
      content: { payload: { source_ref: { entity_id: sourceId }, view: { type: 'table' } }, key_revs: {} },
    }
    return <TableEditorBody key={sourceId} cell={synthetic} sourceMode {...common} />
  }
  if (cell.error && !cell.data) return <div className="aiws-error" role="alert">{t('aiworkspace.data.table.viewReadError', { error: cell.error })}</div>
  if (!cell.data) return <div className="aiws-muted">{t('aiworkspace.data.table.viewLoading')}</div>
  const sourceRef = cell.data.content.payload.source_ref
  if (!sourceRef) return <div className="aiws-warning">{t('aiworkspace.data.table.unbound')}</div>
  return <TableEditorBody key={sourceRef.entity_id} cell={cell.data} {...common} />
}
