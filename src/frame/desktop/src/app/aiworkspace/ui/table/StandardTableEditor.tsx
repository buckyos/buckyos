/* The standard table editor as a host mounts it (UI 框架 PRD §7.1): a Block's editing state, a data-mode tab, a phone's
 * full screen after "展开" — the write lock, the read-only state and the comments come with it. */

import type { EntityEnvelope } from '../../api/types'
import { useDirectReadOnly, useWorkspaceUi, useWriteAccess } from '../../state/hooks'
import { LockScope } from '../blocks/editors'
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
  /** Nothing is edited (a view mode, a sourced table…). */
  readOnly?: boolean
  /** Why, for the title row; worked out from the capability, the source and the connection when absent. */
  readOnlyReason?: string | null
}

export function StandardTableEditor({ cellId, sourceId, source, size = 'embedded', readOnly = false, readOnlyReason }: StandardTableEditorProps) {
  const ui = useWorkspaceUi()
  const access = useWriteAccess(source)
  const readOnlyNow = useDirectReadOnly()
  const lockBlocks = (access.required && !access.held) || readOnlyNow
  return (
    <LockScope entity={readOnly ? undefined : source}>
      <TableViewCell cellId={cellId} sourceId={cellId ? undefined : sourceId} readOnly={readOnly || lockBlocks} readOnlyReason={readOnlyReason} size={size} annotations={ui.annotations}
        onAnnotate={readOnly ? undefined : ui.annotate ?? undefined} onActivateAnnotation={ui.setActiveAnnotation} />
    </LockScope>
  )
}
