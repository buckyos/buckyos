/* The dialogs of the object tree's actions (UI 框架 PRD §5.2): a name for new data (a link's address for linked data),
 * rename, move to a folder, place on a canvas, and the delete confirmation after the reference pre-check (unchanged:
 * still-referenced data is not deleted, references this caller cannot see are only said to exist). Full screen on a
 * phone (§6.1), a modal elsewhere. */

import { createElement, useEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { X } from 'lucide-react'
import { describeError } from '../../api/session'
import { DATA_ID, type EntityEnvelope } from '../../api/types'
import { useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { surfacesOf } from '../canvas/surfaceOps'
import { useOverlayMounted } from '../shell/popover'
import { usePhone, useShell } from '../shell/shellContext'
import { entityLabel, type NewDataKind } from '../sources/dataOps'
import type { DataDialog } from './actions'
import { iconOf, nameOf, typeName } from './model'
import { accepted, createData, createLinked, deleteData, move, moveTargets, placeOnCanvas, rename } from './ops'

function Dialog({ label, testId, onClose, children, actions }: { label: string; testId: string; onClose: () => void; children: ReactNode; actions: ReactNode }) {
  const { t } = useAiwsText()
  const phone = usePhone()
  const close = useRef(onClose)
  useEffect(() => { close.current = onClose })
  useOverlayMounted()
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); close.current() } }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [])
  const host = (document.querySelector('.aiws-root') as HTMLElement | null) ?? document.body
  return createPortal(
    <div className={`aiws-modal-backdrop aiws-dm-backdrop${phone ? ' is-phone' : ''}`} onPointerDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
      <div className="aiws-dialog aiws-dm-dialog" role="dialog" aria-modal="true" aria-label={label} data-testid={testId}>
        <div className="aiws-dialog-head"><b>{label}</b><span className="aiws-grow" /><button type="button" className="aiws-tool" aria-label={t('aiworkspace.common.close')} onClick={onClose}><X size={18} /></button></div>
        <div className="aiws-dm-dialog-body">{children}</div>
        <div className="aiws-dialog-actions">{actions}</div>
      </div>
    </div>,
    host,
  )
}

const DEFAULT_NAME: Record<NewDataKind, string> = {
  table: 'aiworkspace.datamode.default.table', richtext: 'aiworkspace.datamode.default.richtext', record: 'aiworkspace.datamode.default.record', dir: 'aiworkspace.datamode.default.dir',
  link: 'aiworkspace.datamode.default.link', wish: 'aiworkspace.data.default.wish', folder: 'aiworkspace.data.default.folder', note: 'aiworkspace.data.stickyNote',
}
const KIND_LABEL: Record<NewDataKind, string> = {
  table: 'aiworkspace.data.tree.kindTable', richtext: 'aiworkspace.data.tree.kindRichtext', record: 'aiworkspace.data.tree.kindRecord', dir: 'aiworkspace.data.type.dir',
  link: 'aiworkspace.datamode.new.link', wish: 'aiworkspace.data.tree.kindWish', folder: 'aiworkspace.data.tree.kindFolder', note: 'aiworkspace.data.stickyNote',
}

export function DataDialogs({ request, onClose, onCreated }: { request: DataDialog; onClose: () => void; onCreated: (id: string) => void }) {
  switch (request.kind) {
    case 'new': return <NewData request={request} onClose={onClose} onCreated={onCreated} />
    case 'rename': return <Rename entityId={request.entityId} onClose={onClose} />
    case 'move': return <MoveTo entityId={request.entityId} onClose={onClose} />
    case 'place': return <PlaceOnCanvas entityId={request.entityId} onClose={onClose} />
    case 'delete': return <DeleteData request={request} onClose={onClose} />
  }
}

function NewData({ request, onClose, onCreated }: { request: Extract<DataDialog, { kind: 'new' }>; onClose: () => void; onCreated: (id: string) => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const link = request.type === 'link'
  const [title, setTitle] = useState(link ? '' : t(DEFAULT_NAME[request.type]))
  const [address, setAddress] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const input = useRef<HTMLInputElement>(null)
  useEffect(() => { input.current?.focus(); input.current?.select() }, [])
  const folder = store.outline.get(request.place.parentId)
  const where = request.place.afterId
    ? t('aiworkspace.datamode.new.after', { name: nameOf(store, store.outline.get(request.place.afterId), request.place.afterId) })
    : t('aiworkspace.datamode.new.in', { folder: request.place.parentId === DATA_ID ? t('aiworkspace.data.tree.rootLevel') : nameOf(store, folder, request.place.parentId) })
  const submit = async () => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      if (link) {
        const made = await createLinked(store, request.place, address, title)
        onCreated(made.entityId)
        if (made.error) { setError(made.error); setBusy(false); return }
        onClose()
        return
      }
      const id = await createData(store, request.place, request.type as Exclude<NewDataKind, 'link'>, title)
      if (id) { onCreated(id); onClose() } else { setError(t('aiworkspace.datamode.new.failed')); setBusy(false) }
    } catch (failure) {
      setError(describeError(failure))
      setBusy(false)
    }
  }
  const label = t('aiworkspace.datamode.new.title', { kind: t(KIND_LABEL[request.type]) })
  return (
    <Dialog label={label} testId="aiws-dm-new-dialog" onClose={onClose} actions={<>
      <button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
      <button type="submit" form="aiws-dm-new-form" className="is-primary" data-testid="aiws-new-data" disabled={busy || (link && !address.trim())}>{t('aiworkspace.data.tree.create')}</button>
    </>}>
      <form id="aiws-dm-new-form" className="aiws-dm-form" onSubmit={(event) => { event.preventDefault(); void submit() }}>
        <div className="aiws-muted" data-testid="aiws-dm-new-where">{where}</div>
        {link && (
          <label className="aiws-dm-field">{t('aiworkspace.datamode.new.address')}
            <input ref={input} data-testid="aiws-new-link" placeholder={t('aiworkspace.data.tree.linkPlaceholder')} value={address} onChange={(event) => setAddress(event.target.value)} autoComplete="off" spellCheck={false} />
          </label>
        )}
        <label className="aiws-dm-field">{t('aiworkspace.data.tree.titleField')}
          <input ref={link ? undefined : input} data-testid="aiws-dm-new-title" placeholder={link ? t('aiworkspace.data.tree.titlePlaceholder') : undefined} value={title} onChange={(event) => setTitle(event.target.value)} autoComplete="off" />
        </label>
        {error && <div className="aiws-error" role="alert" data-testid="aiws-new-link-error">{error}</div>}
      </form>
    </Dialog>
  )
}

function Rename({ entityId, onClose }: { entityId: string; onClose: () => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const entity = store.outline.get(entityId)
  const [name, setName] = useState(entity ? nameOf(store, entity, entityId) : '')
  const [error, setError] = useState<string | null>(null)
  const input = useRef<HTMLInputElement>(null)
  useEffect(() => { input.current?.focus(); input.current?.select() }, [])
  if (!entity) return null
  const submit = async () => {
    if (!name.trim()) { setError(t('aiworkspace.datamode.rename.empty')); return }
    try {
      const outcome = await rename(store, entity, name)
      if (accepted(outcome)) onClose()
      else setError(t('aiworkspace.datamode.rename.refused', { code: 'code' in outcome ? outcome.code : outcome.status }))
    } catch (failure) { setError(describeError(failure)) }
  }
  return (
    <Dialog label={t('aiworkspace.datamode.rename.title')} testId="aiws-dm-rename-dialog" onClose={onClose} actions={<>
      <button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
      <button type="submit" form="aiws-dm-rename-form" className="is-primary" data-testid="aiws-dm-rename-save">{t('aiworkspace.data.rename')}</button>
    </>}>
      <form id="aiws-dm-rename-form" className="aiws-dm-form" onSubmit={(event) => { event.preventDefault(); void submit() }}>
        <label className="aiws-dm-field">{t('aiworkspace.datamode.prop.name')}
          <input ref={input} data-testid="aiws-dm-rename-input" value={name} onChange={(event) => { setName(event.target.value); setError(null) }} autoComplete="off" />
        </label>
        {entity.title === undefined || entity.title === null ? <div className="aiws-muted">{t('aiworkspace.datamode.rename.pathHint')}</div> : null}
        {error && <div className="aiws-error" role="alert" data-testid="aiws-dm-rename-error">{error}</div>}
      </form>
    </Dialog>
  )
}

function PickList({ items, testPrefix, onPick }: { items: EntityEnvelope[]; testPrefix: string; onPick: (e: EntityEnvelope) => void }) {
  const store = useStore()
  const depth = (e: EntityEnvelope) => Math.max(0, store.outline.ancestors(e.entity_id).length - 2)
  return (
    <ul className="aiws-dm-pick" role="listbox">
      {items.map((e) => (
        <li key={e.entity_id}>
          <button type="button" role="option" aria-selected={false} className="aiws-dm-pick-item" data-testid={`${testPrefix}-${e.entity_id}`} style={{ paddingLeft: 8 + depth(e) * 14 }} onClick={() => onPick(e)}>
            {createElement(iconOf(e), { size: 16, 'aria-hidden': true })}<span>{e.entity_id === DATA_ID ? store.session.info().title : nameOf(store, e, e.entity_id)}</span>
          </button>
        </li>
      ))}
    </ul>
  )
}

function MoveTo({ entityId, onClose }: { entityId: string; onClose: () => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const entity = store.outline.get(entityId)
  if (!entity) return null
  const targets = moveTargets(store, entityId).filter((f) => f.entity_id !== entity.parent_id)
  return (
    <Dialog label={t('aiworkspace.datamode.moveTitle', { name: entityLabel(entity) })} testId="aiws-dm-move-dialog" onClose={onClose} actions={<button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>}>
      {targets.length === 0 ? <div className="aiws-muted">{t('aiworkspace.datamode.noFolders')}</div>
        : <PickList items={targets} testPrefix="aiws-dm-move" onPick={(folder) => { void move(store, entityId, folder.entity_id).then((outcome) => { if (accepted(outcome)) onClose() }) }} />}
    </Dialog>
  )
}

function PlaceOnCanvas({ entityId, onClose }: { entityId: string; onClose: () => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const shell = useShell()
  const entity = store.outline.get(entityId)
  if (!entity) return null
  const surfaces = surfacesOf(store).filter((s) => s.capabilities.includes('structure'))
  const place = async (surface: EntityEnvelope) => {
    const cellId = await placeOnCanvas(store, entity, surface)
    onClose()
    if (cellId) store.notify('info', t('aiworkspace.datamode.placed', { name: entityLabel(entity), canvas: entityLabel(surface) }), {
      label: t('aiworkspace.datamode.goToCanvas'), run: () => { store.userState.set('surface:active', surface.entity_id); shell.setTopMode('canvas') },
    })
  }
  return (
    <Dialog label={t('aiworkspace.datamode.placeTitle', { name: entityLabel(entity) })} testId="aiws-dm-place-dialog" onClose={onClose} actions={<button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>}>
      {surfaces.length === 0 ? <div className="aiws-muted">{t('aiworkspace.datamode.noCanvas')}</div> : <PickList items={surfaces} testPrefix="aiws-dm-place" onPick={(s) => { void place(s) }} />}
    </Dialog>
  )
}

function DeleteData({ request, onClose }: { request: Extract<DataDialog, { kind: 'delete' }>; onClose: () => void }) {
  const { t, list } = useAiwsText()
  const store = useStore()
  const entity = store.outline.get(request.entityId)
  if (!entity) return null
  const { referrers, hidden } = request.check
  const count = store.outline.descendants(entity.entity_id).length
  const blocked = referrers.length > 0 || hidden
  return (
    <Dialog label={t('aiworkspace.data.tree.deleteDialog')} testId="aiws-delete-dialog" onClose={onClose} actions={<>
      <button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
      {!blocked && <button type="button" className="is-danger" data-testid="aiws-delete-confirm" onClick={() => { void deleteData(store, entity).then(onClose) }}>{t('aiworkspace.data.tree.confirmDelete')}</button>}
    </>}>
      <b>{count > 0 ? t('aiworkspace.data.tree.deleteTitleWithChildren', { name: entityLabel(entity), count }) : t('aiworkspace.data.tree.deleteTitle', { name: entityLabel(entity) })}</b>
      <div className="aiws-muted">{typeName(entity)}</div>
      {referrers.length > 0 && (
        <div className="aiws-warning" data-testid="aiws-delete-blocked">{t(hidden ? 'aiworkspace.data.tree.stillReferencedHidden' : 'aiworkspace.data.tree.stillReferenced', {
          names: list(referrers.map((r) => entityLabel(store.outline.get(r.entity_id) ?? { entity_id: r.entity_id } as EntityEnvelope))),
        })}</div>
      )}
      {referrers.length === 0 && hidden && <div className="aiws-warning" data-testid="aiws-delete-blocked">{t('aiworkspace.data.tree.hiddenReferences')}</div>}
    </Dialog>
  )
}
