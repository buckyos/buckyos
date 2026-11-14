/* The left half of the data mode's status bar (UI 框架 PRD §5.5): the current object's type, rows or size, why it is
 * read only, its freshness. A table's current row is not shown: the table editor does not publish it yet. */

import { createElement } from 'react'
import type { EntityEnvelope } from '../../api/types'
import { useReadOnlyReason, useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { FreshnessBadge } from '../sources/FreshnessBadge'
import { iconOf, nameOf, readOnlyReason, typeName } from './model'
import { useObjectSize } from './size'
import { PERMISSIONS_TAB } from './tabs'

export function ObjectInfo({ id, entity, onOpen }: { id: string | null; entity: EntityEnvelope | undefined; onOpen?: () => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const windowReason = useReadOnlyReason()
  const size = useObjectSize(entity)
  const reason = id === PERMISSIONS_TAB ? null : readOnlyReason(store, entity, windowReason)
  const parts = !id ? [] : id === PERMISSIONS_TAB ? [t('aiworkspace.datamode.permissions')] : [typeName(entity), ...(size ? [size.text] : []), ...(reason ? [t('aiworkspace.datamode.info.readOnly', { reason })] : [])]
  const body = (
    <>
      {id && createElement(iconOf(entity, id), { size: 14, 'aria-hidden': true })}
      {id && onOpen && <b className="aiws-dm-info-name">{nameOf(store, entity, id)}</b>}
      <span data-testid="aiws-dm-info-text">{parts.join(' · ') || t('aiworkspace.datamode.info.none')}</span>
      {entity?.derived && <FreshnessBadge entityId={entity.entity_id} showManual={false} />}
    </>
  )
  if (onOpen) return <button type="button" className="aiws-dm-info" data-testid="aiws-dm-info" data-read-only={reason ? 'true' : 'false'} onClick={onOpen} disabled={!id}>{body}</button>
  return <div className="aiws-dm-info" data-testid="aiws-dm-info" data-read-only={reason ? 'true' : 'false'}>{body}</div>
}
