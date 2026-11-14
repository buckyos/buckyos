/* Pieces of the directory editor (框架 PRD §7.4): its menus and dialogs — a panel beside the trigger on wide screens,
 * a sheet from the bottom on a phone, both drawn over the workspace (not inside a transformed canvas Block) — the row
 * pictures, the upload list and the preview. */

import { useCallback, useEffect, useRef, useState, useSyncExternalStore, type ReactNode, type SyntheticEvent } from 'react'
import { createPortal } from 'react-dom'
import { Check, ChevronRight, File as FileIcon, FileText, Film, Folder, FolderOpen, Image as ImageIcon, Music, X, type LucideIcon } from 'lucide-react'
import { ContentPreview } from '../../../../components/ContentPreview'
import type { PreviewSessionContext } from '../../../../components/preview/types'
import { useLoad, useOutlineVersion, useStore, useVersion } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { ControlPopover, type AnchorRect } from '../controls/data/ControlPopover'
import { mediaKindOf, type MediaKind } from '../media/kinds'
import { assetSource } from '../media/source'
import { useOverlayMounted } from '../shell/popover'
import { usePhone } from '../shell/shellContext'
import { entityLabel } from '../sources/dataOps'
import { formatBytes } from './format'
import { entriesAt, placeInside, placeOf, samePlace, type Crumb, type DirectoryContent, type Entry, type FileMember, type Place } from './model'
import type { Resolution } from './ops'
import { uploadQueue, type UploadJob } from './uploads'

const portalRoot = () => (document.querySelector('.aiws-root') as HTMLElement | null) ?? document.body
const stop = (event: SyntheticEvent) => event.stopPropagation()

// ---- menus

export interface DirMenuItem { id: string; label: string; icon?: LucideIcon; danger?: boolean; disabled?: string | false; testId: string; run: () => void }

/** A menu beside its trigger, or a sheet from the bottom on a phone. */
export function DirMenu({ anchor, label, items, onClose, testId }: { anchor: AnchorRect; label: string; items: DirMenuItem[]; onClose: () => void; testId: string }) {
  const phone = usePhone()
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => { ref.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus() }, [])
  return (
    <ControlPopover anchor={anchor} onClose={onClose} label={label} sheet={phone} width={260} testId={testId} side="below">
      <div ref={ref} className="aiws-dir-menu" role="menu" aria-label={label}
        onKeyDown={(event) => {
          if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
          event.preventDefault()
          const buttons = [...(ref.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])]
          const i = buttons.indexOf(document.activeElement as HTMLButtonElement)
          buttons[(i + (event.key === 'ArrowDown' ? 1 : buttons.length - 1)) % buttons.length]?.focus()
        }}>
        <div className="aiws-dir-menu-title">{label}</div>
        {items.map((item) => {
          const Icon = item.icon
          return (
            <button key={item.id} type="button" role="menuitem" className={`aiws-dir-menu-item${item.danger ? ' is-danger' : ''}`} data-testid={item.testId}
              disabled={Boolean(item.disabled)} title={item.disabled || undefined} onClick={() => { onClose(); item.run() }}>
              <span className="aiws-dir-menu-icon" aria-hidden="true">{Icon && <Icon size={18} />}</span>
              <span className="aiws-dir-menu-label">{item.label}{item.disabled && <small>{item.disabled}</small>}</span>
            </button>
          )
        })}
      </div>
    </ControlPopover>
  )
}

// ---- dialogs

/** A dialog over the workspace; a sheet from the bottom on a phone. Events stay inside it (a canvas under it takes none). */
export function DirDialog({ label, testId, onClose, children, actions }: { label: string; testId: string; onClose: () => void; children: ReactNode; actions: ReactNode }) {
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
  return createPortal(
    <div className={`aiws-modal-backdrop aiws-dir-backdrop${phone ? ' is-sheet' : ''}`} onClick={stop} onDoubleClick={stop} onKeyDown={stop} onContextMenu={stop} onWheel={stop}
      onPointerDown={(event) => { event.stopPropagation(); if (event.target === event.currentTarget) onClose() }}>
      <div className="aiws-dialog aiws-dir-dialog" role="dialog" aria-modal="true" aria-label={label} data-testid={testId}>
        <div className="aiws-dir-dialog-head">
          <h3>{label}</h3>
          <button type="button" className="aiws-dir-icon" aria-label={t('aiworkspace.common.close')} title={t('aiworkspace.common.close')} onClick={onClose}><X size={18} aria-hidden="true" /></button>
        </div>
        <div className="aiws-dir-dialog-body">{children}</div>
        <div className="aiws-dialog-actions aiws-dir-dialog-actions">{actions}</div>
      </div>
    </div>,
    portalRoot(),
  )
}

/** A name to give (rename, a new subdirectory): `check` says what is wrong with it, or null. */
export function NameDialog({ title, initial, confirm, check, onSubmit, onClose, testId = 'aiws-directory-name-dialog' }: {
  title: string; initial: string; confirm: string; check: (name: string) => string | null; onSubmit: (name: string) => Promise<void>; onClose: () => void; testId?: string
}) {
  const { t } = useAiwsText()
  const [name, setName] = useState(initial)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const input = useRef<HTMLInputElement>(null)
  useEffect(() => {
    const el = input.current
    if (!el) return
    el.focus()
    // the name before its extension, as a file manager selects it
    const dot = initial.lastIndexOf('.')
    el.setSelectionRange(0, dot > 0 ? dot : initial.length)
  }, [initial])
  const submit = async () => {
    const value = name.trim()
    const problem = check(value)
    if (problem) { setError(problem); return }
    setBusy(true)
    try { await onSubmit(value); onClose() } catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); setBusy(false) }
  }
  return (
    <DirDialog label={title} testId={testId} onClose={onClose} actions={<>
      <button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
      <button type="submit" form={`${testId}-form`} className="is-primary" data-testid="aiws-directory-save" disabled={busy}>{confirm}</button>
    </>}>
      <form id={`${testId}-form`} className="aiws-dir-form" onSubmit={(event) => { event.preventDefault(); void submit() }}>
        <label className="aiws-field">{t('aiworkspace.directory.name')}
          <input ref={input} data-testid="aiws-directory-name" aria-label={t('aiworkspace.directory.newName')} value={name} enterKeyHint="done" autoComplete="off" spellCheck={false}
            onChange={(event) => { setName(event.target.value); setError(null) }} aria-invalid={error ? true : undefined} />
        </label>
        {error && <div className="aiws-error" role="alert" data-testid="aiws-directory-name-error">{error}</div>}
      </form>
    </DirDialog>
  )
}

export function ConfirmDialog({ title, body, confirm, onConfirm, onClose }: { title: string; body: string; confirm: string; onConfirm: () => Promise<void>; onClose: () => void }) {
  const { t } = useAiwsText()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const go = async () => {
    setBusy(true)
    try { await onConfirm(); onClose() } catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); setBusy(false) }
  }
  return (
    <DirDialog label={title} testId="aiws-directory-confirm" onClose={onClose} actions={<>
      <button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
      <button type="button" className="is-danger" data-testid="aiws-directory-confirm-ok" disabled={busy} autoFocus onClick={() => { void go() }}>{confirm}</button>
    </>}>
      <p className="aiws-muted">{body}</p>
      {error && <div className="aiws-error" role="alert">{error}</div>}
    </DirDialog>
  )
}

/** "There is already one of that name": replace, keep both or skip — for this one, or for the rest as well. */
export function ConflictDialog({ names, onDone }: { names: string[]; onDone: (resolutions: Record<string, Resolution> | null) => void }) {
  const { t } = useAiwsText()
  const [index, setIndex] = useState(0)
  const [all, setAll] = useState(false)
  const [chosen, setChosen] = useState<Record<string, Resolution>>({})
  const name = names[index]
  const rest = names.length - index - 1
  const pick = (resolution: Resolution) => {
    const next = { ...chosen }
    for (const n of all ? names.slice(index) : [name]) next[n] = resolution
    if (all || rest === 0) { onDone(next); return }
    setChosen(next)
    setIndex(index + 1)
  }
  return (
    <DirDialog label={t('aiworkspace.directory.conflict.title', { name })} testId="aiws-directory-conflict" onClose={() => onDone(null)} actions={<>
      <button type="button" data-testid="aiws-directory-conflict-skip" onClick={() => pick('skip')}>{t('aiworkspace.directory.conflict.skip')}</button>
      <button type="button" data-testid="aiws-directory-conflict-keep" onClick={() => pick('keep')}>{t('aiworkspace.directory.conflict.keep')}</button>
      <button type="button" className="is-primary" data-testid="aiws-directory-conflict-replace" onClick={() => pick('replace')}>{t('aiworkspace.directory.conflict.replace')}</button>
    </>}>
      <p className="aiws-muted">{t('aiworkspace.directory.conflict.body')}</p>
      {rest > 0 && <label className="aiws-dir-check"><input type="checkbox" data-testid="aiws-directory-conflict-all" checked={all} onChange={(event) => setAll(event.target.checked)} />{t('aiworkspace.directory.conflict.all', { count: rest })}</label>}
    </DirDialog>
  )
}

/** Where to move rows: a folder browser starting at the current place; other directories of the workspace at the top. */
export function MoveDialog({ rootId, rootName, crumbs: startCrumbs, from, moving, onPick, onClose }: {
  rootId: string; rootName: string; crumbs: Crumb[]; from: Place; moving: Entry[]; onPick: (target: Place) => Promise<void>; onClose: () => void
}) {
  const { t, compare } = useAiwsText()
  const store = useStore()
  useOutlineVersion()
  const [root, setRoot] = useState<{ id: string; name: string } | null>({ id: rootId, name: rootName })
  const [crumbs, setCrumbs] = useState<Crumb[]>(startCrumbs)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const place = root ? placeOf(root.id, crumbs) : null
  const version = useVersion(`e:${place?.entityId ?? ''}`)
  const load = useCallback(() => (place ? store.session.read<DirectoryContent>(place.entityId) : Promise.resolve(null)), [store, place?.entityId]) // eslint-disable-line react-hooks/exhaustive-deps
  const read = useLoad(load, version)
  const current = place && read.data?.entity_id === place.entityId ? read.data : undefined
  const movingDirs = new Set(moving.flatMap((entry) => (entry.kind === 'dir' ? [entry.member.entity_id] : [])))
  const movingIds = new Set(moving.map((entry) => entry.id))
  const folders = current && place ? entriesAt(current.content.payload.members ?? {}, place.prefix, compare)
    .filter((entry) => entry.kind !== 'file' && !(samePlace(place, from) && movingIds.has(entry.id)) && !(entry.kind === 'dir' && movingDirs.has(entry.member.entity_id))) : []
  const others = store.outline.all().filter((e) => e.type_id === 'buckyos.dir' && !e.deleted && !e.degraded && e.entity_id !== rootId && !movingDirs.has(e.entity_id) && e.capabilities.includes('read'))
  const target = place ? store.outline.get(place.entityId) : undefined
  const reason = !place ? t('aiworkspace.directory.move.choose')
    : samePlace(place, from) ? t('aiworkspace.directory.move.same')
      : !target?.capabilities.includes('update') || target.source ? t('aiworkspace.directory.move.noPermission') : null
  const go = async () => {
    if (!place || reason) return
    setBusy(true)
    try { await onPick(place); onClose() } catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); setBusy(false) }
  }
  return (
    <DirDialog label={t('aiworkspace.directory.move.title', { count: moving.length })} testId="aiws-directory-move-dialog" onClose={onClose} actions={<>
      <button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
      <button type="button" className="is-primary" data-testid="aiws-directory-move-here" disabled={busy || Boolean(reason)} title={reason ?? undefined} onClick={() => { void go() }}>{t('aiworkspace.directory.move.here')}</button>
    </>}>
      <nav className="aiws-dir-crumbs is-dialog" aria-label={t('aiworkspace.directory.crumbs')}>
        <button type="button" className="aiws-dir-crumb" data-testid="aiws-directory-move-top" aria-current={root === null ? 'location' : undefined} onClick={() => { setRoot(null); setCrumbs([]) }}>{t('aiworkspace.directory.move.all')}</button>
        {root && <><ChevronRight size={14} aria-hidden="true" /><button type="button" className="aiws-dir-crumb" aria-current={crumbs.length === 0 ? 'location' : undefined} onClick={() => setCrumbs([])}>{root.name}</button></>}
        {crumbs.map((crumb, i) => <span key={i} className="aiws-dir-crumb-step"><ChevronRight size={14} aria-hidden="true" />
          <button type="button" className="aiws-dir-crumb" aria-current={i === crumbs.length - 1 ? 'location' : undefined} onClick={() => setCrumbs(crumbs.slice(0, i + 1))}>{crumb.name}</button></span>)}
      </nav>
      <div className="aiws-dir-picklist" role="list">
        {root === null && [{ id: rootId, name: rootName }, ...others.map((e) => ({ id: e.entity_id, name: entityLabel(e) }))].map((item) => (
          <button key={item.id} type="button" role="listitem" className="aiws-dir-pick" data-testid="aiws-directory-move-root" data-entity-id={item.id} onClick={() => { setRoot(item); setCrumbs([]) }}>
            <FolderOpen size={20} aria-hidden="true" /><span>{item.name}</span><ChevronRight size={16} aria-hidden="true" />
          </button>
        ))}
        {root !== null && folders.map((entry) => (
          <button key={entry.id} type="button" role="listitem" className="aiws-dir-pick" data-testid="aiws-directory-move-folder" data-name={entry.name}
            onClick={() => { if (placeInside(place!, entry)) setCrumbs([...crumbs, entry.kind === 'dir' ? { kind: 'dir', name: entry.name, entityId: entry.member.entity_id } : { kind: 'folder', name: entry.name }]) }}>
            <Folder size={20} aria-hidden="true" /><span>{entry.name}</span><ChevronRight size={16} aria-hidden="true" />
          </button>
        ))}
        {root !== null && current && folders.length === 0 && <div className="aiws-muted aiws-dir-pick-empty">{t('aiworkspace.directory.move.noFolders')}</div>}
        {root !== null && read.error && <div className="aiws-error" role="alert">{read.error}</div>}
      </div>
      {reason && root !== null && <div className="aiws-muted" data-testid="aiws-directory-move-reason">{reason}</div>}
      {error && <div className="aiws-error" role="alert">{error}</div>}
    </DirDialog>
  )
}

/** An existing directory as a member: choose it, then its name here (its own name by default). */
export function ExistingDialog({ exclude, check, onAdd, onClose }: { exclude: ReadonlySet<string>; check: (name: string) => string | null; onAdd: (directoryId: string, name: string) => Promise<void>; onClose: () => void }) {
  const { t, compare } = useAiwsText()
  const store = useStore()
  useOutlineVersion()
  const [chosen, setChosen] = useState<string | null>(null)
  const [name, setName] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const directories = store.outline.all().filter((e) => e.type_id === 'buckyos.dir' && !e.deleted && !e.degraded && !exclude.has(e.entity_id) && e.capabilities.includes('read'))
    .sort((a, b) => compare(entityLabel(a), entityLabel(b)))
  const add = async () => {
    if (!chosen) return
    const problem = check(name.trim())
    if (problem) { setError(problem); return }
    setBusy(true)
    try { await onAdd(chosen, name.trim()); onClose() } catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); setBusy(false) }
  }
  return (
    <DirDialog label={t('aiworkspace.directory.existing.title')} testId="aiws-directory-add-dir" onClose={onClose} actions={<>
      <button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
      <button type="button" className="is-primary" data-testid="aiws-directory-add" disabled={busy || !chosen} onClick={() => { void add() }}>{t('aiworkspace.directory.add')}</button>
    </>}>
      <div className="aiws-dir-picklist" role="listbox" aria-label={t('aiworkspace.directory.selectDirectory')}>
        {directories.map((e) => (
          <button key={e.entity_id} type="button" role="option" aria-selected={chosen === e.entity_id} className="aiws-dir-pick" data-testid="aiws-directory-dir-target" data-entity-id={e.entity_id}
            onClick={() => { setChosen(e.entity_id); setName(entityLabel(e)); setError(null) }}>
            <FolderOpen size={20} aria-hidden="true" /><span>{entityLabel(e)}</span>{chosen === e.entity_id && <Check size={16} aria-hidden="true" />}
          </button>
        ))}
        {directories.length === 0 && <div className="aiws-muted aiws-dir-pick-empty">{t('aiworkspace.directory.existing.none')}</div>}
      </div>
      {chosen && <label className="aiws-field">{t('aiworkspace.directory.memberName')}
        <input data-testid="aiws-directory-dir-name" value={name} onChange={(event) => { setName(event.target.value); setError(null) }} />
      </label>}
      {error && <div className="aiws-error" role="alert">{error}</div>}
    </DirDialog>
  )
}

// ---- row pictures

const KIND_ICON: Record<MediaKind, LucideIcon> = { image: ImageIcon, video: Film, audio: Music, pdf: FileText, file: FileIcon }
/** Pictures up to this size are drawn as thumbnails; larger ones (and everything else) show their kind's icon. */
const THUMB_MAX_BYTES = 8 * 1024 * 1024
const thumbs = new Map<string, Promise<string>>()

function thumbUrl(read: () => Promise<Blob>, objectId: string, mediaType: string): Promise<string> {
  let url = thumbs.get(objectId)
  if (!url) {
    url = read().then((blob) => URL.createObjectURL(blob.type === mediaType ? blob : new Blob([blob], { type: mediaType })))
    url.catch(() => thumbs.delete(objectId))
    thumbs.set(objectId, url)
    // a bounded cache: the oldest pictures go (their URLs with them)
    if (thumbs.size > 240) {
      const [oldest, pending] = thumbs.entries().next().value as [string, Promise<string>]
      thumbs.delete(oldest)
      void pending.then((u) => URL.revokeObjectURL(u), () => undefined)
    }
  }
  return url
}

/** The picture of a row: a picture file's thumbnail once it is on screen, else its kind's icon; a folder icon. */
export function Thumb({ kind, member, name }: { kind: Entry['kind']; member?: FileMember; name: string }) {
  const store = useStore()
  const ref = useRef<HTMLSpanElement>(null)
  const [url, setUrl] = useState<string | null>(null)
  const media = member ? mediaKindOf(member.media_type, name) : 'file'
  const objectId = member?.object_id
  const wanted = kind === 'file' && media === 'image' && Boolean(objectId) && (member?.size ?? 0) <= THUMB_MAX_BYTES
  useEffect(() => {
    const el = ref.current
    if (!wanted || !el || !objectId) return
    let live = true
    const observer = new IntersectionObserver((records) => {
      if (!records.some((r) => r.isIntersecting)) return
      observer.disconnect()
      thumbUrl(() => store.session.fetchAsset(objectId), objectId, member?.media_type ?? 'image/png').then((u) => { if (live) setUrl(u) }, () => undefined)
    })
    observer.observe(el)
    return () => { live = false; observer.disconnect() }
  }, [wanted, objectId, store, member?.media_type])
  const Icon = kind === 'file' ? KIND_ICON[media] : kind === 'dir' ? FolderOpen : Folder
  return (
    <span ref={ref} className={`aiws-dir-thumb is-${kind === 'file' ? media : 'folder'}`} aria-hidden="true">
      {url ? <img src={url} alt="" draggable={false} /> : <Icon size={22} />}
    </span>
  )
}

// ---- uploads

/** The uploads started from this editor: progress, cancel, and why one failed (a size limit says which). */
export function UploadList({ rootId }: { rootId: string }) {
  const { t } = useAiwsText()
  const store = useStore()
  const queue = uploadQueue(store)
  useSyncExternalStore(queue.subscribe, queue.snapshot)
  const jobs = queue.list(rootId)
  if (jobs.length === 0) return null
  const status = (job: UploadJob) => {
    switch (job.state) {
      case 'waiting': return t('aiworkspace.directory.upload.waiting')
      case 'sending': return job.sent > 0 && job.total > 0 ? t('aiworkspace.directory.upload.sending', { percent: Math.min(99, Math.floor((job.sent / job.total) * 100)), sent: formatBytes(job.sent), total: formatBytes(job.total) }) : t('aiworkspace.directory.upload.preparing')
      case 'saving': return t('aiworkspace.directory.upload.saving')
      case 'done': return t('aiworkspace.directory.upload.done', { name: job.added ?? job.name })
      case 'cancelled': return t('aiworkspace.directory.upload.cancelled')
      case 'failed': return job.error ?? ''
    }
  }
  return (
    <ul className="aiws-dir-uploads" data-testid="aiws-directory-uploads" aria-label={t('aiworkspace.directory.upload.list')}>
      {jobs.map((job) => {
        const running = job.state === 'waiting' || job.state === 'sending' || job.state === 'saving'
        return (
          <li key={job.id} className={`aiws-dir-upload is-${job.state}`} data-testid="aiws-directory-upload-job" data-name={job.name} data-state={job.state}>
            <span className="aiws-dir-upload-text">
              <span className="aiws-dir-upload-name">{job.name}</span>
              <span className={job.state === 'failed' ? 'aiws-dir-upload-error' : 'aiws-muted'} role={job.state === 'failed' ? 'alert' : undefined} data-testid="aiws-directory-upload-status">{status(job)}</span>
              {running && <progress max={job.total || 1} value={job.state === 'saving' ? job.total || 1 : job.sent} aria-label={t('aiworkspace.directory.upload.progress', { name: job.name })} />}
            </span>
            <button type="button" className="aiws-dir-icon" data-testid={running ? 'aiws-directory-upload-cancel' : 'aiws-directory-upload-dismiss'}
              aria-label={running ? t('aiworkspace.directory.upload.cancel', { name: job.name }) : t('aiworkspace.directory.upload.dismiss')} title={running ? t('aiworkspace.directory.upload.cancel', { name: job.name }) : t('aiworkspace.directory.upload.dismiss')}
              disabled={job.state === 'saving'} onClick={() => (running ? queue.cancel(job.id) : queue.dismiss(job.id))}><X size={16} aria-hidden="true" /></button>
          </li>
        )
      })}
    </ul>
  )
}

// ---- preview

export interface PreviewFile { key: string; name: string; member: FileMember }

/** The preview of the files at a place: beside or under the list on a wide screen, the whole screen on a phone; previous
 * / next go through the same files. `start` is the file opened from the list; `onShown` follows the preview's own paging. */
export function PreviewPane({ files, start, allowExport, onShown, onClose }: { files: PreviewFile[]; start: string; allowExport: boolean; onShown: (key: string) => void; onClose: () => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const phone = usePhone()
  const [shown, setShown] = useState(start)
  const startIndex = Math.max(0, files.findIndex((file) => file.key === start))
  // the list is fixed while this preview is open (its key remounts it): paging must not reset it
  const [session] = useState<PreviewSessionContext & { kind: 'list' }>(() => ({
    kind: 'list', sessionId: `aiws-dir:${store.session.workspaceId}:${start}`, currentIndex: startIndex,
    items: files.map((file) => ({ id: file.key, title: file.name, source: assetSource(store, { object_id: file.member.object_id, media_type: file.member.media_type, file_name: file.name }, file.name) })),
  }))
  const file = files.find((item) => item.key === shown) ?? files[startIndex]
  if (!file) return null
  const body = (
    <div className={`aiws-dir-preview aiws-preview-host${phone ? ' is-full' : ''}`} data-testid="aiws-directory-preview" data-name={file.name} role={phone ? 'dialog' : 'region'} aria-modal={phone ? true : undefined}
      aria-label={t('aiworkspace.directory.previewOf', { name: file.name })} onPointerDown={phone ? stop : undefined} onKeyDown={phone ? stop : undefined}>
      {/* the preview's own bar names the file, pages and closes (its exit) */}
      <ContentPreview source={session.items[session.currentIndex].source} session={session} uiMode="visible" allowExport={allowExport} prefetchAdjacent={false}
        onItemChanged={(item) => { setShown(item.id); onShown(item.id) }} onRequestExit={onClose} className="aiws-dir-preview-body" />
    </div>
  )
  return phone ? createPortal(body, portalRoot()) : body
}
