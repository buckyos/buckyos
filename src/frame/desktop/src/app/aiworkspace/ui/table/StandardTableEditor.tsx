/* The standard table editor as a host mounts it (UI 框架 PRD §7.1): a Block's editing state, a data-mode tab, a phone's
 * full screen after "展开" — the write lock, the read-only state and the comments come with it. */

import type { EntityEnvelope } from '../../api/types'
import { useDirectReadOnly, useEntity, useWorkspaceUi, useWriteAccess } from '../../state/hooks'
import { TableViewCell } from '../TableViewCell'
import type { TableEditorSize } from './model'

export interface StandardTableEditorProps {
  /** A table-view Block: its view (columns, saved filter and sort) is shown and written. Omit for the table itself. */
  cellId?: string
  /** The TableSource. */
  sourceId: string
  /** Its outline entry (capabilities, write lock); undefined while unknown. */
  source: EntityEnvelope | undefined
  /** 'embedded' (a bounded box, default), 'tab' or 'fullscreen' (fill the host, which must give the editor a height). */
  size?: TableEditorSize
  compact?: boolean
  /** Nothing is edited (a view mode, a sourced table…). */
  readOnly?: boolean
  /** Why, for the title row; worked out from the capability, the source and the connection when absent. */
  readOnlyReason?: string | null
  annotationParentId: string | null
  onStartAnnotation?: () => void
}

export function StandardTableEditor({ cellId, sourceId, source, size = 'embedded', compact = false, readOnly = false, readOnlyReason, annotationParentId, onStartAnnotation }: StandardTableEditorProps) {
  const ui = useWorkspaceUi()
  const lockSource = compact || readOnly ? undefined : source
  const access = useWriteAccess(lockSource)
  const readOnlyNow = useDirectReadOnly()
  const folder = useEntity(annotationParentId ?? '')
  const annotateAt = ui.annotateAt
  const lockBlocks = (access.required && !access.held) || readOnlyNow
  return (
    <TableViewCell cellId={cellId} sourceId={cellId ? undefined : sourceId} lockSource={lockSource}
      awaitingLock={Boolean(lockSource && !readOnlyNow && lockBlocks)} readOnly={compact || readOnly || lockBlocks}
      readOnlyReason={readOnlyReason} compact={compact} size={size} annotations={ui.annotations}
      onAnnotate={annotationParentId && annotateAt && !readOnlyNow && folder?.capabilities.includes('comment')
        ? (anchor) => { onStartAnnotation?.(); annotateAt(anchor, annotationParentId) } : undefined} onActivateAnnotation={ui.setActiveAnnotation} />
  )
}
