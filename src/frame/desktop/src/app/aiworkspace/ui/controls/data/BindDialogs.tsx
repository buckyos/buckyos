/* The host's default dialogs of 控件 on_bind TODO §2.2 — pick one data (or a new one), pick several, confirm — and the
 * placeholder of a control that has no data yet. `bindDialogs` opens them on the one `BindDialogHost` the workspace page
 * mounts; drawn over the whole workspace (also when asked from inside a Block), a sheet from the bottom on a phone. */

import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import { Check, Folder, Plus, Table2 } from 'lucide-react'
import { aiwsT } from '../../../i18n'
import { useEntity, useReadOnlyReason, useStore } from '../../../state/hooks'
import { useAiwsText } from '../../../useAiwsText'
import { Modal } from '../../shell/dialogs'
import { bindControl, findData } from './binding'
import { bindDialogs, registerDialogHost, type Ask } from './dialogBus'

/** Mounted once by the workspace page. */
export function BindDialogHost() {
  const [ask, setAsk] = useState<Ask | null>(null)
  useEffect(() => registerDialogHost((next) => setAsk(next)), [])
  if (!ask) return null
  const done = () => setAsk(null)
  if (ask.kind === 'one') return <PickOne ask={ask} onDone={(v) => { done(); ask.resolve(v) }} />
  if (ask.kind === 'many') return <PickMany ask={ask} onDone={(v) => { done(); ask.resolve(v) }} />
  return <Confirm ask={ask} onDone={(v) => { done(); ask.resolve(v) }} />
}

function Sheet({ title, onClose, children }: { title: string; onClose: () => void; children: React.ReactNode }) {
  return createPortal(
    <Modal label={title} testId="aiws-source-chooser" className="aiws-bind-dialog" onClose={onClose}>{children}</Modal>,
    (document.querySelector('.aiws-root') as HTMLElement | null) ?? document.body,
  )
}

function DataRow({ id, title, place, folder, selected, onClick }: { id: string; title: string; place: string; folder: boolean; selected?: boolean; onClick: () => void }) {
  const { t } = useAiwsText()
  return (
    <button type="button" role="listitem" className={`aiws-source-item${selected ? ' is-selected' : ''}`} aria-pressed={selected} data-testid={`aiws-source-${id}`} onClick={onClick}>
      {selected ? <Check size={16} aria-hidden="true" /> : folder ? <Folder size={16} aria-hidden="true" /> : <Table2 size={16} aria-hidden="true" />}
      <span><span className="aiws-source-name">{title}</span>{place && <span className="aiws-muted">{t('aiworkspace.datactl.source.in', { place })}</span>}</span>
    </button>
  )
}

function PickOne({ ask, onDone }: { ask: Extract<Ask, { kind: 'one' }>; onDone: (v: string | { new: true } | null) => void }) {
  const store = useStore()
  const { t } = useAiwsText()
  const data = findData(store, ask.query)
  const title = t('aiworkspace.datactl.source.choose', { name: ask.title })
  return (
    <Sheet title={title} onClose={() => onDone(null)}>
      <b>{title}</b>
      {ask.note && <div className="aiws-muted">{ask.note}</div>}
      <div className="aiws-source-list" role="list">
        {data.map((d) => <DataRow key={d.entity_id} id={d.entity_id} title={d.title ?? d.entity_id} place={d.place ?? ''} folder={d.type_id === 'buckyos.dir'} selected={d.entity_id === ask.current} onClick={() => onDone(d.entity_id)} />)}
        {ask.allowNew && (
          <button type="button" role="listitem" className="aiws-source-item" data-testid="aiws-source-new" onClick={() => onDone({ new: true })}>
            <Plus size={16} aria-hidden="true" /><span className="aiws-source-name">{t('aiworkspace.datactl.source.new')}</span>
          </button>
        )}
      </div>
      <div className="aiws-dialog-actions"><button type="button" data-testid="aiws-bind-cancel" onClick={() => onDone(null)}>{t('aiworkspace.common.cancel')}</button></div>
    </Sheet>
  )
}

function PickMany({ ask, onDone }: { ask: Extract<Ask, { kind: 'many' }>; onDone: (v: (string | { new: true })[] | null) => void }) {
  const store = useStore()
  const { t } = useAiwsText()
  const data = findData(store, ask.query)
  const [chosen, setChosen] = useState<Set<string>>(() => new Set(ask.preselect === 'all' ? data.map((d) => d.entity_id) : ask.preselect === 'none' ? [] : ask.preselect))
  const [fresh, setFresh] = useState(false)
  const toggle = (id: string) => setChosen((s) => { const n = new Set(s); if (n.has(id)) n.delete(id); else n.add(id); return n })
  const title = t('aiworkspace.datactl.source.choose', { name: ask.title })
  return (
    <Sheet title={title} onClose={() => onDone(null)}>
      <b>{title}</b>
      {ask.note && <div className="aiws-muted">{ask.note}</div>}
      <div className="aiws-source-list" role="list">
        {data.map((d) => <DataRow key={d.entity_id} id={d.entity_id} title={d.title ?? d.entity_id} place={d.place ?? ''} folder={d.type_id === 'buckyos.dir'} selected={chosen.has(d.entity_id)} onClick={() => toggle(d.entity_id)} />)}
        {data.length === 0 && !ask.allowNew && <div className="aiws-muted">{t('aiworkspace.datactl.source.none')}</div>}
        {ask.allowNew && (
          <button type="button" role="listitem" className={`aiws-source-item${fresh ? ' is-selected' : ''}`} aria-pressed={fresh} data-testid="aiws-source-new" onClick={() => setFresh((v) => !v)}>
            {fresh ? <Check size={16} aria-hidden="true" /> : <Plus size={16} aria-hidden="true" />}<span className="aiws-source-name">{t('aiworkspace.datactl.source.new')}</span>
          </button>
        )}
      </div>
      <div className="aiws-dialog-actions">
        <button type="button" className="is-primary" data-testid="aiws-bind-confirm" onClick={() => onDone([...data.map((d) => d.entity_id).filter((id) => chosen.has(id)), ...(fresh ? [{ new: true as const }] : [])])}>{t('aiworkspace.datactl.source.done')}</button>
        <button type="button" data-testid="aiws-bind-cancel" onClick={() => onDone(null)}>{t('aiworkspace.common.cancel')}</button>
      </div>
    </Sheet>
  )
}

function Confirm({ ask, onDone }: { ask: Extract<Ask, { kind: 'confirm' }>; onDone: (v: boolean) => void }) {
  const { t } = useAiwsText()
  return (
    <Sheet title={ask.title} onClose={() => onDone(false)}>
      <b>{ask.title}</b>
      <div>{ask.text}</div>
      <div className="aiws-dialog-actions">
        <button type="button" className="is-primary" data-testid="aiws-bind-confirm" onClick={() => onDone(true)}>{t('aiworkspace.common.ok')}</button>
        <button type="button" data-testid="aiws-bind-cancel" onClick={() => onDone(false)}>{t('aiworkspace.common.cancel')}</button>
      </div>
    </Sheet>
  )
}

/** A control without the data it needs: says so and, for who may change it, binds it the way an insertion does. */
export function UnboundSource({ containerId }: { containerId: string }) {
  const store = useStore()
  const { t } = useAiwsText()
  const container = useEntity(containerId)
  const readOnly = useReadOnlyReason()
  if (!container) return null
  const canBind = !readOnly && container.capabilities.includes('update')
  const start = async () => {
    const result = await bindControl(store, containerId, 'insert', bindDialogs, aiwsT('aiworkspace.datactl.source.bindLabel', { name: container.title ?? '' }))
    if (result !== 'done' && result !== 'cancelled') store.notify('error', aiwsT('aiworkspace.datactl.rejected', { reason: result }))
  }
  return (
    <div className="aiws-dc-empty">
      <div>{t('aiworkspace.datactl.unbound')}</div>
      {canBind && <button type="button" className="aiws-dc-bind" data-testid={`aiws-dc-bind-${containerId}`} onPointerDown={(event) => event.stopPropagation()} onClick={() => void start()}>{t('aiworkspace.datactl.source.bind')}</button>}
    </div>
  )
}
