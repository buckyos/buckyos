/* The left half of the data mode's status bar (UI 框架 PRD §5.5): the current object's type, rows or size, why it is
 * read only, its freshness; a table's current row instead of its size while its editor is on one (ui/table/current.ts). */

import { createElement } from 'react'
import type { EntityEnvelope } from '../../api/types'
import { useReadOnlyReason, useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { FreshnessBadge } from '../sources/FreshnessBadge'
import { useTableCurrent } from '../table/current'
import { iconOf, nameOf, readOnlyReason, typeName } from './model'
import { useObjectSize } from './size'
import { PERMISSIONS_TAB } from './tabs'

export function ObjectInfo({ id, entity, onOpen }: { id: string | null; entity: EntityEnvelope | undefined; onOpen?: () => void }) {
  const { t, num } = useAiwsText()
  const store = useStore()
  const windowReason = useReadOnlyReason()
  const size = useObjectSize(entity)
  const row = useTableCurrent(entity?.type_id === 'buckyos.table-source' ? entity.entity_id : null)
  const reason = id === PERMISSIONS_TAB ? null : readOnlyReason(store, entity, windowReason)
  const where = row ? t('aiworkspace.tablecurrent.row', { index: num(row.index), total: num(row.total) }) : null
  const parts = !id ? [] : id === PERMISSIONS_TAB ? [t('aiworkspace.datamode.permissions')]
    : [typeName(entity), ...(where ? [where] : size ? [size.text] : []), ...(reason ? [t('aiworkspace.datamode.info.readOnly', { reason })] : [])]
  const body = (
    <>
      {id && createElement(iconOf(entity, id), { size: 14, 'aria-hidden': true })}
      {id && onOpen && <b className="aiws-dm-info-name">{nameOf(store, entity, id)}</b>}
      <span data-testid="aiws-dm-info-text" title={row?.title ? t('aiworkspace.tablecurrent.record', { name: row.title }) : undefined}
        data-record-id={row?.recordId}>{parts.join(' · ') || t('aiworkspace.datamode.info.none')}</span>
      {entity?.derived && <FreshnessBadge entityId={entity.entity_id} showManual={false} />}
    </>
  )
  if (onOpen) return <button type="button" className="aiws-dm-info" data-testid="aiws-dm-info" data-read-only={reason ? 'true' : 'false'} onClick={onOpen} disabled={!id}>{body}</button>
  return <div className="aiws-dm-info" data-testid="aiws-dm-info" data-read-only={reason ? 'true' : 'false'}>{body}</div>
}
