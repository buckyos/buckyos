import clsx from 'clsx'
import { Check, ChevronRight, FolderPlus, HardDrive, History, Laptop, Library } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { contentDescriptor, matchSelector, type ContentSelector, type SelectDialogResult, type SelectedContent, type ZoneContent } from 'buckyos/content'
import { useI18n } from '../../i18n/provider'
import { normalizeCyfsPath } from '../../components/preview/session'
import { useMobileBackHandler } from '../../desktop/windows/MobileNavContext'
import { useCollections } from '../../app/filebrowser/data/collectionDirectory'
import type { FileItem } from '../../app/filebrowser/data/FolderReader'
import { folderOps } from '../../app/filebrowser/data/folderOps'
import { installFileBrowserData } from '../../app/filebrowser/data/install'
import { resolveReader } from '../../app/filebrowser/data/readerRegistry'
import { entryNameSchema } from '../../app/filebrowser/data/schemas'
import { useSidebarDfs } from '../../app/filebrowser/data/sidebarSources'
import { toUiError } from '../../app/filebrowser/data/state'
import { collectionUrl, crumbsForUrl, dfsPathOf, normalizeUrl, parentUrl } from '../../app/filebrowser/data/urls'
import { useFolderList } from '../../app/filebrowser/data/useFolderList'
import { NamePromptDialog } from '../../app/filebrowser/dialogs/NamePromptDialog'
import { formatBytes, kindIcon } from '../../app/filebrowser/fileDisplay'
import type { FileEntry, SortKey } from '../../app/filebrowser/types'
import { checkSelectDialogResult, type SelectDialogInput } from './schemas'

const HOME = '/home'
const RECENT_URL = 'view://recent'
const lastDirKey = (callerId: string) => `sysdlg.select.lastDir:${callerId}`

const refOf = (path: string) => ({ kind: 'cyfs-path' as const, path: normalizeCyfsPath(path) })
const splitPath = (path: string) => ({ parent: path.slice(0, path.lastIndexOf('/')) || '/', name: path.slice(path.lastIndexOf('/') + 1) })
const accepted = (accept: ContentSelector[], name: string, size?: number, mime?: string) =>
  !accept.length || accept.some((selector) => matchSelector(selector, contentDescriptor(refOf(`/${name}`), { name, size, mime })) > 0)

function zoneContent(entry: FileEntry): ZoneContent {
  const source = refOf(entry.path)
  const folder = entry.kind === 'folder'
  return {
    origin: 'zone', kind: folder ? 'folder' : 'file', source,
    ...(entry.objId ? { objId: entry.objId } : {}),
    hints: folder ? { name: entry.name } : { name: entry.name, mime: contentDescriptor(source, { name: entry.name }).mime, size: entry.sizeBytes },
  }
}

/** `<input accept>` for the selectors; undefined when they cannot be expressed as MIME patterns and extensions. */
function inputAccept(accept: ContentSelector[]): string | undefined {
  const out = new Set<string>()
  for (const selector of accept) {
    const mimes = selector.mime === undefined ? [] : Array.isArray(selector.mime) ? selector.mime : [selector.mime]
    const exts = selector.ext ?? []
    if ((!mimes.length && !exts.length) || mimes.some((mime) => mime === '*/*' || mime.includes('*+'))) return undefined
    mimes.forEach((mime) => out.add(mime.split(';')[0].trim()))
    exts.forEach((ext) => out.add(ext.startsWith('.') ? ext : `.${ext}`))
  }
  return out.size ? [...out].join(',') : undefined
}

const canPickLocalFolder = () => 'webkitdirectory' in document.createElement('input') && !window.matchMedia?.('(pointer: coarse)').matches

interface StartLocation { url: string; preselect?: FileEntry }

async function resolveStart(input: SelectDialogInput, callerId: string): Promise<StartLocation> {
  const folderAt = async (path: string): Promise<StartLocation | null> => {
    if (path === '/') return { url: normalizeUrl('/') }
    const { parent, name } = splitPath(path)
    const entry = await folderOps().statEntry(parent, name).catch(() => null)
    if (!entry) return null
    return entry.kind === 'folder' ? { url: normalizeUrl(path) } : { url: normalizeUrl(parent), preselect: { ...entry, path } }
  }
  const start = input.start ? dfsPathOf(normalizeUrl(normalizeCyfsPath(input.start.path).slice('cyfs://'.length))) : null
  let last: string | null = null
  try { last = window.localStorage.getItem(lastDirKey(callerId)) } catch { /* no storage */ }
  for (const path of [start, start ? null : last]) {
    if (!path || !path.startsWith('/')) continue
    const found = await folderAt(path)
    if (found) return found
  }
  return { url: normalizeUrl(HOME) }
}

export interface SelectDialogProps {
  input: SelectDialogInput
  /** Whose "last used folder" to start from. */
  callerId?: string
  /** Narrow layout: locations collapse into a drop-down above the list. */
  compact?: boolean
  onDone: (result: SelectDialogResult) => void
}

/**
 * The system file picker. A pure component: parameters in, one `SelectDialogResult` out. It does not know whether
 * it was opened inside the Desktop, by the shell for an App window, or by the embed page of a standalone App tab.
 */
export function SelectDialog(props: SelectDialogProps) {
  const { t } = useI18n()
  const callerId = props.callerId ?? 'desktop'
  const [start, setStart] = useState<StartLocation | null>(null)
  useEffect(() => {
    let cancelled = false
    installFileBrowserData()
    void resolveStart(props.input, callerId).then((location) => { if (!cancelled) setStart(location) })
    return () => { cancelled = true }
  }, [props.input, callerId])
  if (!start) return <div className="flex h-[min(60vh,520px)] items-center justify-center text-sm text-[color:var(--cp-muted)]" role="status">{t('sysdlg.select.loading', 'Loading…')}</div>
  return <SelectBrowser {...props} callerId={callerId} start={start} />
}

function SelectBrowser({ input, callerId, compact, onDone, start }: SelectDialogProps & { callerId: string; start: StartLocation }) {
  const { t } = useI18n()
  const { mode } = input
  const selectable = useCallback((entry: FileEntry) =>
    mode === 'open' && entry.kind !== 'folder' && entry.path.startsWith('/') && !entry.link?.broken && accepted(input.accept, entry.name, entry.sizeBytes),
  [mode, input.accept])
  const [url, setUrl] = useState(start.url)
  const [selected, setSelected] = useState<Map<string, FileEntry>>(() => new Map(start.preselect && selectable(start.preselect) ? [[start.preselect.path, start.preselect]] : []))
  const [name, setName] = useState(input.suggestedName || (mode === 'save' && start.preselect ? start.preselect.name : ''))
  const [conflict, setConflict] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [creating, setCreating] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  const localInput = useRef<HTMLInputElement>(null)
  const objIds = useRef(new Map<string, string>())

  // By name where the location can; a collection or a view keeps its own order.
  const sortKey = useMemo<SortKey>(() => {
    const reader = resolveReader(url)
    try { return reader.capabilities.sortKeys.includes('name') ? 'name' : reader.capabilities.defaultSortKey }
    finally { reader.dispose() }
  }, [url])
  const list = useFolderList(url, sortKey, 'asc')
  const dfs = useSidebarDfs()
  const collections = useCollections()

  const path = list.capabilities.kind === 'folder' ? dfsPathOf(url) : null
  const ready = list.status === 'ready'
  const isExcluded = useCallback((target: string) => input.exclude.some((dir) => target === dir || target.startsWith(`${dir}/`)), [input.exclude])
  const excluded = path !== null && isExcluded(path)
  const writable = list.capabilities.acceptsContent
  const items = list.loadedItems()
  const rows = useMemo(() => mode === 'folder' ? items.filter((item) => item.entry.kind === 'folder') : items, [items, mode])
  useEffect(() => { for (const { entry } of items) if (entry.objId) objIds.current.set(entry.path, entry.objId) }, [items])
  const more = list.hasMore || (list.totalCount ?? 0) > list.loadedCount
  const loadMore = () => list.ensureRange(list.loadedCount, list.loadedCount + 199)
  const full = input.multiple && selected.size >= input.limit

  useEffect(() => { root.current?.focus({ preventScroll: true }) }, [])

  const finish = useCallback((result: SelectDialogResult) => {
    try {
      const checked = checkSelectDialogResult(result, input)
      const dir = dfsPathOf(url)
      if (checked.action !== 'cancelled' && dir !== null && !(checked.action === 'selected' && checked.items[0].origin === 'local')) {
        try { window.localStorage.setItem(lastDirKey(callerId), dir) } catch { /* no storage */ }
      }
      onDone(checked)
    } catch (error) { setNotice(toUiError(error).fallback) }
  }, [callerId, input, onDone, url])
  const cancel = useCallback(() => onDone({ action: 'cancelled' }), [onDone])
  const navigate = (next: string) => { setUrl(normalizeUrl(next)); setNotice(null); setConflict(null) }
  const up = parentUrl(url)
  useMobileBackHandler(useCallback(() => { if (url === start.url || !up) cancel(); else setUrl(up) }, [cancel, start.url, up, url]))

  const toggle = (entry: FileEntry) => {
    setNotice(null)
    setSelected((prev) => {
      if (!input.multiple) return new Map([[entry.path, entry]])
      const next = new Map(prev)
      if (next.has(entry.path)) next.delete(entry.path)
      else if (next.size < input.limit) next.set(entry.path, entry)
      return next
    })
  }
  const activate = (item: FileItem) => {
    const { entry } = item
    if (entry.kind === 'folder') navigate(item.ref?.refPath ?? entry.path)
    else if (mode === 'save') { setName(entry.name); setConflict(null) }
    else if (selectable(entry)) toggle(entry)
  }

  const nameCheck = entryNameSchema.safeParse(name)
  const canConfirm = !busy && (
    mode === 'open' ? selected.size > 0
      : mode === 'folder' ? path !== null && ready && !excluded && (!input.writable || writable)
        : path !== null && ready && !excluded && writable && nameCheck.success)
  const confirm = async (overwrite = false) => {
    if (!canConfirm) return
    if (mode === 'open') return finish({ action: 'selected', items: [...selected.values()].map(zoneContent) })
    if (path === null) return
    if (mode === 'folder') {
      const folderName = splitPath(path).name
      const objId = objIds.current.get(path)
      return finish({ action: 'selected', items: [{ origin: 'zone', kind: 'folder', source: refOf(path), ...(objId ? { objId } : {}), hints: folderName ? { name: folderName } : {} }] })
    }
    if (!nameCheck.success) return
    const fileName = nameCheck.data
    if (overwrite) return finish({ action: 'save', parent: refOf(path), name: fileName, overwrite: true })
    setBusy(true); setNotice(null)
    try {
      // Asked of the server, not of the pages loaded so far.
      const existing = await folderOps().statEntry(path, fileName)
      if (!existing) finish({ action: 'save', parent: refOf(path), name: fileName, overwrite: false })
      else if (existing.kind === 'folder') setNotice(t('sysdlg.select.existsFolder', 'A folder named “{{name}}” already exists here. Choose another name.', { name: fileName }))
      else setConflict(fileName)
    } catch (error) {
      setNotice(t('sysdlg.select.checkFailed', 'Could not check this location: {{error}}', { error: toUiError(error).fallback }))
    } finally { setBusy(false) }
  }

  const pickedLocal = (files: File[]) => {
    if (!files.length) { if (mode === 'folder') setNotice(t('sysdlg.select.localEmptyFolder', 'That folder has no files that can be read.')); return }
    if (mode === 'folder') {
      const folderName = (files[0].webkitRelativePath || files[0].name).split('/')[0]
      return finish({ action: 'selected', items: [{ origin: 'local', kind: 'folder', name: folderName, files: files.map((file) => ({ file, relativePath: file.webkitRelativePath || `${folderName}/${file.name}` })) }] })
    }
    if (files.length > input.limit) return setNotice(t('sysdlg.select.localTooMany', 'You chose {{count}} files; up to {{limit}} are allowed.', { count: files.length, limit: input.limit }))
    const rejected = files.filter((file) => !accepted(input.accept, file.name, file.size, file.type || undefined))
    if (rejected.length) return setNotice(t('sysdlg.select.localNotAccepted', 'These files cannot be chosen here: {{names}}', { names: rejected.slice(0, 5).map((file) => file.name).join(', ') + (rejected.length > 5 ? '…' : '') }))
    const picked: SelectedContent[] = files.map((file) => ({ origin: 'local', kind: 'file', file, hints: { name: file.name, mime: file.type || contentDescriptor(refOf(`/${file.name}`), { name: file.name }).mime, size: file.size } }))
    finish({ action: 'selected', items: picked })
  }

  const locations = useMemo(() => {
    // Roots that are not real folders (a collection shown as a root) are places to pick from, not to write to.
    const out: Array<{ key: string; label: string; url: string; icon: ReactNode }> = (dfs.state.data ?? []).map((node) => ({
      key: node.id, url: normalizeUrl(node.path), icon: <HardDrive size={15} aria-hidden="true" />,
      label: node.kind === 'home' ? t('sysdlg.select.home', 'My files') : node.name,
    })).filter((location) => mode === 'open' || dfsPathOf(location.url) !== null)
    if (mode === 'open') {
      out.push({ key: 'recent', label: t('sysdlg.select.recent', 'Recent'), url: RECENT_URL, icon: <History size={15} aria-hidden="true" /> })
      for (const collection of collections) out.push({ key: `c-${collection.id}`, label: collection.title, url: collectionUrl(collection.id), icon: <Library size={15} aria-hidden="true" /> })
    }
    return out
  }, [collections, dfs.state.data, mode, t])
  const activeLocation = [...locations].sort((a, b) => b.url.length - a.url.length).find((location) => url === location.url || url.startsWith(`${location.url}/`))
  const localEntry = input.allowLocal && (mode === 'open' || (mode === 'folder' && canPickLocalFolder()))
  const openLocal = () => { setNotice(null); localInput.current?.click() }

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key !== 'Escape' || creating) return
    event.stopPropagation()
    if (conflict) setConflict(null)
    else cancel()
  }
  const signedOut = list.error?.name === 'UNAUTHENTICATED'
  const status = list.status === 'loading' ? t('sysdlg.select.loading', 'Loading…')
    : list.status === 'error' ? list.error?.message ?? ''
      : notice ?? (mode === 'open' && selected.size ? t('sysdlg.select.selectedCount', '{{count}} selected', { count: selected.size }) : '')
  const confirmLabel = input.confirmLabel ?? t(`sysdlg.select.confirm.${mode}`)
  const buttonClass = 'min-h-10 rounded-full px-4 text-sm font-medium disabled:opacity-50'

  return (
    <div ref={root} tabIndex={-1} onKeyDown={onKeyDown} data-testid="select-dialog" data-mode={mode}
      className={clsx('flex min-h-0 flex-col gap-3 text-[color:var(--cp-text)] outline-none', compact ? 'h-[min(70dvh,560px)]' : 'h-[min(60vh,520px)]')}>
      <div className="sr-only" role="status" aria-live="polite" data-testid="select-status">{status}</div>
      {localEntry && <input ref={localInput} type="file" hidden data-testid="select-local-input" multiple={input.multiple || mode === 'folder'} accept={mode === 'open' ? inputAccept(input.accept) : undefined}
        {...(mode === 'folder' ? { webkitdirectory: '' } : {})}
        onChange={(event) => { const files = Array.from(event.target.files ?? []); event.target.value = ''; pickedLocal(files) }} />}

      <div className={clsx('flex min-h-0 flex-1 gap-3', compact && 'flex-col')}>
        {compact ? (
          <div className="flex gap-2">
            <select aria-label={t('sysdlg.select.locations', 'Locations')} data-testid="select-locations" value={activeLocation?.url ?? ''} onChange={(event) => { if (event.target.value) navigate(event.target.value) }}
              className="min-h-10 min-w-0 flex-1 rounded-xl border border-[color:var(--cp-border)] bg-transparent px-2 text-sm">
              {!activeLocation && <option value="">{t('sysdlg.select.locations', 'Locations')}</option>}
              {locations.map((location) => <option key={location.key} value={location.url}>{location.label}</option>)}
            </select>
            {localEntry && <button type="button" data-testid="select-local" onClick={openLocal} className={clsx(buttonClass, 'flex items-center gap-1 border border-[color:var(--cp-border)]')}><Laptop size={15} aria-hidden="true" />{t('sysdlg.select.local', 'This device')}</button>}
          </div>
        ) : (
          <nav aria-label={t('sysdlg.select.locations', 'Locations')} className="desktop-scrollbar w-44 shrink-0 overflow-y-auto" data-testid="select-locations">
            {localEntry && <button type="button" data-testid="select-local" title={t('sysdlg.select.localHint', 'Opens the file picker of this device')} onClick={openLocal}
              className="mb-1 flex min-h-9 w-full items-center gap-2 rounded-xl border border-[color:var(--cp-border)] px-2 text-left text-sm"><Laptop size={15} aria-hidden="true" /><span className="truncate">{t('sysdlg.select.local', 'This device')}</span></button>}
            {locations.map((location) => <button type="button" key={location.key} aria-current={activeLocation?.key === location.key ? 'location' : undefined} onClick={() => navigate(location.url)}
              className={clsx('flex min-h-9 w-full items-center gap-2 rounded-xl px-2 text-left text-sm', activeLocation?.key === location.key && 'bg-[color:color-mix(in_srgb,var(--cp-accent)_14%,transparent)] font-medium')}>{location.icon}<span className="truncate">{location.label}</span></button>)}
            {dfs.state.status === 'error' && <button type="button" className="min-h-9 px-2 text-left text-sm underline" onClick={dfs.retry}>{t('sysdlg.select.retry', 'Retry')}</button>}
          </nav>
        )}

        <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-2">
          <nav aria-label={t('sysdlg.select.path', 'Path')} className="flex flex-wrap items-center gap-0.5 text-sm" data-testid="select-crumbs">
            {crumbsForUrl(url, list.meta?.title).map((crumb, index, all) => <span key={crumb.url} className="flex items-center">
              {index > 0 && <ChevronRight size={14} aria-hidden="true" className="text-[color:var(--cp-muted)]" />}
              <button type="button" disabled={index === all.length - 1} aria-current={index === all.length - 1 ? 'page' : undefined} onClick={() => navigate(crumb.url)}
                className="min-h-8 max-w-40 truncate rounded-lg px-1.5 disabled:font-medium">{crumb.label === 'root' && index === 0 ? '/' : crumb.label}</button>
            </span>)}
            {mode !== 'open' && path !== null && writable && <button type="button" data-testid="select-new-folder" onClick={() => setCreating(true)} className="ml-auto flex min-h-8 items-center gap-1 rounded-lg px-2 text-sm"><FolderPlus size={15} aria-hidden="true" />{t('sysdlg.select.newFolder', 'New folder')}</button>}
          </nav>

          <div role="listbox" aria-label={t('sysdlg.select.items', 'Files and folders')} aria-multiselectable={input.multiple || undefined} aria-busy={list.status === 'loading'} data-testid="select-list"
            className="desktop-scrollbar min-h-0 flex-1 overflow-y-auto rounded-xl border border-[color:var(--cp-border)]"
            onScroll={(event) => { const el = event.currentTarget; if (more && list.status !== 'loading' && el.scrollTop + el.clientHeight > el.scrollHeight - 120) loadMore() }}>
            {list.status === 'error' ? (
              <div role="alert" className="flex flex-col items-start gap-2 p-4 text-sm" data-testid="select-error">
                <p>{signedOut ? t('sysdlg.select.signInHint', 'Your session has ended. Sign in again, then retry — your selection is kept.') : list.error?.message}</p>
                <div className="flex gap-2">
                  {signedOut && <a className="underline" href={new URL('/login', window.location.origin).href} target="_blank" rel="noreferrer">{t('sysdlg.select.signIn', 'Sign in again')}</a>}
                  <button type="button" className="underline" onClick={() => list.reload()}>{t('sysdlg.select.retry', 'Retry')}</button>
                </div>
              </div>
            ) : <>
              {rows.map((item) => {
                const { entry } = item
                const folder = entry.kind === 'folder'
                const isSelected = selected.has(entry.path)
                const usable = folder ? !isExcluded(entry.path) : mode === 'save' || (selectable(entry) && (isSelected || !full))
                const reason = usable ? undefined : folder || mode !== 'open' ? t('sysdlg.select.excluded', 'This folder cannot be the destination') : selectable(entry) ? t('sysdlg.select.limitReached', 'You can choose up to {{limit}} items', { limit: input.limit }) : t('sysdlg.select.notAccepted', 'This type cannot be chosen here')
                return <button type="button" role="option" key={item.key} aria-selected={isSelected} aria-disabled={!usable} title={reason} data-testid="select-row" data-name={entry.name} data-kind={folder ? 'folder' : 'file'}
                  onClick={() => { if (usable) activate(item) }}
                  onDoubleClick={() => { if (mode === 'open' && !input.multiple && selectable(entry)) finish({ action: 'selected', items: [zoneContent(entry)] }) }}
                  className={clsx('flex min-h-11 w-full items-center gap-2 border-b border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] px-3 text-left text-sm',
                    isSelected && 'bg-[color:color-mix(in_srgb,var(--cp-accent)_16%,transparent)]', !usable && 'cursor-not-allowed opacity-45', mode === 'save' && !folder && 'text-[color:var(--cp-muted)]')}>
                  {mode === 'open' && input.multiple && !folder && <span aria-hidden="true" className={clsx('flex size-4 shrink-0 items-center justify-center rounded border', isSelected ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent)] text-white' : 'border-[color:var(--cp-border)]')}>{isSelected && <Check size={12} />}</span>}
                  {kindIcon(entry.kind)}
                  <span className="min-w-0 flex-1 truncate">{entry.name}</span>
                  {!folder && entry.sizeBytes !== undefined && <span className="shrink-0 text-xs text-[color:var(--cp-muted)]">{formatBytes(entry.sizeBytes)}</span>}
                  {folder && <ChevronRight size={15} aria-hidden="true" className="shrink-0 text-[color:var(--cp-muted)]" />}
                </button>
              })}
              {list.status === 'loading' && <p className="p-4 text-sm text-[color:var(--cp-muted)]">{t('sysdlg.select.loading', 'Loading…')}</p>}
              {ready && !rows.length && !more && <p className="p-4 text-sm text-[color:var(--cp-muted)]" data-testid="select-empty">{mode === 'folder' ? t('sysdlg.select.emptyFolders', 'No folders here') : t('sysdlg.select.empty', 'Nothing here')}</p>}
              {ready && more && <button type="button" data-testid="select-load-more" className="min-h-11 w-full text-sm underline" onClick={loadMore}>{t('sysdlg.select.loadMore', 'Load more')}</button>}
            </>}
          </div>
        </div>
      </div>

      {mode !== 'open' && path !== null && ready && !writable && (mode === 'save' || input.writable) && <p role="alert" className="text-sm text-[color:var(--cp-danger)]" data-testid="select-readonly">{t('sysdlg.select.readOnly', 'You cannot save to this location. Choose another folder.')}</p>}
      {excluded && <p role="alert" className="text-sm text-[color:var(--cp-danger)]" data-testid="select-excluded">{t('sysdlg.select.excluded', 'This folder cannot be the destination')}</p>}
      {notice && <p role="alert" className="text-sm text-[color:var(--cp-danger)]" data-testid="select-notice">{notice}</p>}
      {conflict && <div role="alertdialog" aria-label={t('sysdlg.select.replace', 'Replace')} className="flex flex-wrap items-center gap-2 rounded-xl border border-[color:var(--cp-border)] p-3 text-sm" data-testid="select-conflict">
        <p className="min-w-0 flex-1">{t('sysdlg.select.exists', '“{{name}}” already exists here. Replace it?', { name: conflict })}</p>
        <button type="button" className={clsx(buttonClass, 'border border-[color:var(--cp-border)]')} onClick={() => setConflict(null)}>{t('sysdlg.select.keep', 'Keep editing')}</button>
        <button type="button" autoFocus data-testid="select-replace" className={clsx(buttonClass, 'bg-[color:var(--cp-danger)] text-white')} onClick={() => void confirm(true)}>{t('sysdlg.select.replace', 'Replace')}</button>
      </div>}

      <form className="flex flex-wrap items-center gap-2" onSubmit={(event) => { event.preventDefault(); void confirm() }}>
        {mode === 'save' && <input aria-label={t('sysdlg.select.fileName', 'File name')} data-testid="select-name" value={name} maxLength={255} enterKeyHint="done" autoComplete="off" spellCheck={false}
          aria-invalid={!!name && !nameCheck.success} onChange={(event) => { setName(event.target.value); setConflict(null); setNotice(null) }}
          className="min-h-10 min-w-0 flex-[1_1_12rem] rounded-xl border border-[color:var(--cp-border)] bg-transparent px-3 text-sm" placeholder={t('sysdlg.select.fileName', 'File name')} />}
        {mode === 'open' && <p className="min-w-0 flex-1 truncate text-sm text-[color:var(--cp-muted)]" data-testid="select-count">
          {full ? t('sysdlg.select.limitReached', 'You can choose up to {{limit}} items', { limit: input.limit }) : selected.size ? t('sysdlg.select.selectedCount', '{{count}} selected', { count: selected.size }) : ''}</p>}
        {mode === 'folder' && <span className="flex-1" />}
        <button type="button" data-testid="select-cancel" className={clsx(buttonClass, 'border border-[color:var(--cp-border)]')} onClick={cancel}>{t('common.cancel', 'Cancel')}</button>
        <button type="submit" data-testid="select-confirm" disabled={!canConfirm || !!conflict} className={clsx(buttonClass, 'bg-[color:var(--cp-accent)] text-white')}>{confirmLabel}</button>
      </form>

      <NamePromptDialog request={creating && path !== null ? { title: t('sysdlg.select.newFolder', 'New folder'), label: t('sysdlg.select.folderName', 'Folder name'), submitLabel: t('sysdlg.select.create', 'Create'), schema: entryNameSchema,
        onSubmit: async (folderName) => { try { await folderOps().createFolder(path, folderName) } catch (error) { throw new Error(toUiError(error).fallback) } navigate(`${path === '/' ? '' : path}/${folderName}`) } } : null} onClose={() => setCreating(false)} />
    </div>
  )
}
