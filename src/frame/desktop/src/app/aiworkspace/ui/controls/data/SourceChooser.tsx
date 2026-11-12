/* The chooser of a data control's table and the unbound control's placeholder (rules in source.ts). */

import { useState } from 'react'
import { createPortal } from 'react-dom'
import { Plus, Table2 } from 'lucide-react'
import { randomId } from '../../../api/ids'
import type { EntityEnvelope, Operation } from '../../../api/types'
import { aiwsT } from '../../../i18n'
import { useEntity, useReadOnlyReason, useStore } from '../../../state/hooks'
import { useAiwsText } from '../../../useAiwsText'
import { Modal } from '../../shell/dialogs'
import { controlRegistry } from '../registry'
import { bindOps, newSourceOps, placeOf, sourceTables, surfaceFolderOf } from './source'

/** Which table: an id, or `new` (when `allowNew`). Drawn over the whole workspace, also when asked from inside a Block. */
export function SourceChooser({ title, note, tables, allowNew = true, onPick, onClose }: {
  title: string; note?: string; tables: EntityEnvelope[]; allowNew?: boolean; onPick: (choice: string) => void; onClose: () => void
}) {
  const store = useStore()
  const { t } = useAiwsText()
  return createPortal(
    <Modal label={title} testId="aiws-source-chooser" onClose={onClose}>
      <b>{title}</b>
      {note && <div className="aiws-muted">{note}</div>}
      <div className="aiws-source-list" role="list">
        {tables.map((table) => {
          const place = placeOf(store, table)
          return (
            <button key={table.entity_id} type="button" role="listitem" className="aiws-source-item" data-testid={`aiws-source-${table.entity_id}`} onClick={() => onPick(table.entity_id)}>
              <Table2 size={16} aria-hidden="true" />
              <span><span className="aiws-source-name">{table.title ?? table.name ?? table.entity_id}</span>{place && <span className="aiws-muted">{t('aiworkspace.datactl.source.in', { place })}</span>}</span>
            </button>
          )
        })}
        {allowNew && (
          <button type="button" role="listitem" className="aiws-source-item" data-testid="aiws-source-new" onClick={() => onPick('new')}>
            <Plus size={16} aria-hidden="true" /><span className="aiws-source-name">{t('aiworkspace.datactl.source.new')}</span>
          </button>
        )}
      </div>
      <div className="aiws-dialog-actions"><button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button></div>
    </Modal>,
    (document.querySelector('.aiws-root') as HTMLElement | null) ?? document.body,
  )
}

/** A data control without its table: says so and, for who may change it, binds the table the same way an insertion does. */
export function UnboundSource({ containerId }: { containerId: string }) {
  const store = useStore()
  const { t } = useAiwsText()
  const container = useEntity(containerId)
  const readOnly = useReadOnlyReason()
  const [tables, setTables] = useState<EntityEnvelope[] | null>(null)
  const definition = controlRegistry.get(container?.kind)
  const source = definition?.source
  if (!container || !source) return null
  const title = container.title ?? definition.title
  const bind = async (choice: string) => {
    setTables(null)
    const tableId = choice === 'new' ? randomId('t') : choice
    const operations: Operation[] = []
    if (choice === 'new') {
      const folder = surfaceFolderOf(store, containerId)
      if (!folder) return
      operations.push(...newSourceOps(store, source, folder, tableId, title))
    }
    operations.push(...bindOps(containerId, { [source.binding]: tableId }))
    const outcome = await store.submit({ editId: `bind:${containerId}`, label: aiwsT('aiworkspace.datactl.source.bindLabel', { name: title }), operations })
    if (outcome.status === 'rejected') store.notify('error', aiwsT('aiworkspace.datactl.rejected', { reason: outcome.errors?.[0]?.detail ?? outcome.detail ?? outcome.code }))
  }
  const start = () => {
    const found = sourceTables(store, source.schemaId)
    if (found.length === 0) void bind('new')
    else if (found.length === 1) void bind(found[0].entity_id)
    else setTables(found)
  }
  const canBind = !readOnly && container.capabilities.includes('update')
  return (
    <div className="aiws-dc-empty">
      <div>{t('aiworkspace.datactl.unbound')}</div>
      {canBind && <button type="button" className="aiws-dc-bind" data-testid={`aiws-dc-bind-${containerId}`} onPointerDown={(event) => event.stopPropagation()} onClick={start}>{t('aiworkspace.datactl.source.bind')}</button>}
      {tables && <SourceChooser title={t('aiworkspace.datactl.source.title', { name: title })} note={t('aiworkspace.datactl.source.note')} tables={tables} onPick={(choice) => void bind(choice)} onClose={() => setTables(null)} />}
    </div>
  )
}
