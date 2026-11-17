import clsx from 'clsx'
import { Check, ChevronRight, FolderPlus, HardDrive, History, House, Library, MonitorSmartphone, Search, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { contentDescriptor, matchSelector, mediaTypeFromExtension, type ContentSelector, type SelectDialogResult, type SelectedContent, type ZoneContent } from 'buckyos/content'
import { useI18n } from '../../i18n/provider'
import { normalizeCyfsPath } from '../../components/preview/session'
import { FileThumb } from '../../components/preview/FileThumb'
import { useMobileBackHandler } from '../../desktop/windows/MobileNavContext'
import { useCollections } from '../../app/filebrowser/data/collectionDirectory'
import type { FileItem } from '../../app/filebrowser/data/FolderReader'
import { folderOps } from '../../app/filebrowser/data/folderOps'
import { installFileBrowserData } from '../../app/filebrowser/data/install'
import { resolveReader } from '../../app/filebrowser/data/readerRegistry'
import { entryNameSchema } from '../../app/filebrowser/data/schemas'
import { useSidebarDfs } from '../../app/filebrowser/data/sidebarSources'
import { myHome, useMyHome } from '../../app/filebrowser/data/myHome'
import { toUiError } from '../../app/filebrowser/data/state'
import { collectionUrl, crumbsForUrl, dfsPathOf, normalizeUrl, parentUrl } from '../../app/filebrowser/data/urls'
import { useFolderList } from '../../app/filebrowser/data/useFolderList'
import { useSearch } from '../../app/filebrowser/data/search'
import { NamePromptDialog } from '../../app/filebrowser/dialogs/NamePromptDialog'
import { formatBytes } from '../../app/filebrowser/fileDisplay'
import type { FileEntry, SortDir, SortKey } from '../../app/filebrowser/types'
import { checkSelectDialogResult, type SelectDialogInput } from './schemas'
import '../../components/preview/files.css'

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

const isMediaType = (mime: string) => mime.startsWith('image/') || mime.startsWith('video/')
/** True when every selector only takes images or videos: the picker then lays the items out as a grid of thumbnails. */
function mediaOnly(accept: ContentSelector[]): boolean {
  return accept.length > 0 && accept.every((selector) => {
    if (selector.objType || selector.schema) return false
    if (selector.mime !== undefined) return (Array.isArray(selector.mime) ? selector.mime : [selector.mime]).every((mime) => isMediaType(mime.toLowerCase()))
    return !!selector.ext?.length && selector.ext.every((ext) => isMediaType(mediaTypeFromExtension(ext.replace(/^\./, '')) ?? ''))
  })
}

/** The picker's thumbnail: shared with Files' icon view; small list rows only ask for images and videos. */
function Thumb({ entry, size, box, scope }: { entry: FileEntry; size: number; box: number; scope: 'media' | 'all' }) {
  const source = useMemo(() => entry.path.startsWith('/') ? refOf(entry.path) : null, [entry.path])
  return <FileThumb entry={entry} source={source} iconSize={size} box={box} scope={scope} marks={scope === 'all'} />
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
  // Where a pick most often starts from: what the user touched lately. Choosing a folder or a place to save starts at home.
  return { url: input.mode === 'open' ? RECENT_URL : normalizeUrl((await myHome()).path) }
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
  const [query, setQuery] = useState('')
  const root = useRef<HTMLDivElement>(null)
  const localInput = useRef<HTMLInputElement>(null)
  const objIds = useRef(new Map<string, string>())
  // The last two items clicked: a double click confirms only when both clicks hit the same item (on touch, a tap on a
  // folder and then on the file that takes its place arrives as a double click too).
  const clicks = useRef<[string | null, string | null]>([null, null])

  // By name in a folder; a view or a collection keeps its own order (Recent: newest first).
  const [sortKey, sortDir] = useMemo<[SortKey, SortDir]>(() => {
    const reader = resolveReader(url)
    try {
      const { kind, sortKeys, defaultSortKey } = reader.capabilities
      const key = kind === 'folder' && sortKeys.includes('name') ? 'name' : defaultSortKey
      return [key, key === 'modified' ? 'desc' : 'asc']
    } finally { reader.dispose() }
  }, [url])
  const list = useFolderList(url, sortKey, sortDir)
  const dfs = useSidebarDfs()
  const home = useMyHome()
  const collections = useCollections()

  const path = list.capabilities.kind === 'folder' ? dfsPathOf(url) : null
  const ready = list.status === 'ready'
  const isExcluded = useCallback((target: string) => input.exclude.some((dir) => target === dir || target.startsWith(`${dir}/`)), [input.exclude])
  const excluded = path !== null && isExcluded(path)
  const writable = list.capabilities.acceptsContent
  const items = list.loadedItems()
  // Search covers every place the user can read, whatever folder is open.
  const searching = query.trim() !== ''
  const search = useSearch(query, undefined, mode === 'folder' ? 'folder' : '')
  const hits = useMemo<FileItem[]>(() => (search.state.data?.items ?? []).map(({ entry }) => ({ key: `hit:${entry.id}:${entry.path}`, entry })), [search.state.data])
  const shown = searching ? hits : items
  const rows = useMemo(() => mode === 'folder' ? shown.filter((item) => item.entry.kind === 'folder') : shown, [shown, mode])
  useEffect(() => { for (const { entry } of shown) if (entry.objId) objIds.current.set(entry.path, entry.objId) }, [shown])
  const more = searching ? !!search.state.data?.nextCursor : list.hasMore || (list.totalCount ?? 0) > list.loadedCount
  const loading = searching ? search.state.status === 'loading' : list.status === 'loading'
  const loadMore = () => { if (searching) search.loadMore(); else list.ensureRange(list.loadedCount, list.loadedCount + 199) }
  const grid = mode === 'open' && mediaOnly(input.accept)
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
  const navigate = (next: string) => { setUrl(normalizeUrl(next)); setNotice(null); setConflict(null); setQuery('') }
  const up = parentUrl(url)
  useMobileBackHandler(useCallback(() => { if (searching) setQuery(''); else if (url === start.url || !up) cancel(); else setUrl(up) }, [cancel, searching, start.url, up, url]))

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
    else if (mode === 'save') {
      // A file found by search names the target, in its own folder.
      if (searching) navigate(splitPath(entry.path).parent)
      setName(entry.name); setConflict(null)
    }
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
    const out: Array<{ key: string; label: string; url: string; icon: ReactNode }> = [
      ...(home ? [{ key: 'my-files', url: normalizeUrl(home.path), icon: <House size={16} aria-hidden="true" className="shrink-0 text-[color:var(--cp-muted)]" />, label: t('sysdlg.select.home', 'My files') }] : []),
      ...(dfs.state.data ?? []).map((node) => ({ key: node.id, url: normalizeUrl(node.path), icon: <HardDrive size={16} aria-hidden="true" className="shrink-0 text-[color:var(--cp-muted)]" />, label: node.name })),
    ].filter((location) => mode === 'open' || dfsPathOf(location.url) !== null)
    if (mode === 'open') {
      out.push({ key: 'recent', label: t('sysdlg.select.recent', 'Recent'), url: RECENT_URL, icon: <History size={16} aria-hidden="true" className="shrink-0 text-[color:var(--cp-muted)]" /> })
      for (const collection of collections) out.push({ key: `c-${collection.id}`, label: collection.title, url: collectionUrl(collection.id), icon: <Library size={16} aria-hidden="true" className="shrink-0 text-[color:var(--cp-muted)]" /> })
    }
    return out
  }, [collections, dfs.state.data, home, mode, t])
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
  const searchStatus = search.state.status === 'loading' ? t('sysdlg.select.searching', 'Searching…')
    : search.state.status === 'error' ? search.state.error?.fallback ?? ''
      : search.state.status === 'success' ? t('sysdlg.select.searchCount', '{{count}} found', { count: rows.length }) : ''
  const status = searching ? notice ?? searchStatus
    : list.status === 'loading' ? t('sysdlg.select.loading', 'Loading…')
      : list.status === 'error' ? list.error?.message ?? ''
        : notice ?? (mode === 'open' && selected.size ? t('sysdlg.select.selectedCount', '{{count}} selected', { count: selected.size }) : '')
  const confirmLabel = input.confirmLabel ?? t(`sysdlg.select.confirm.${mode}`)
  const buttonClass = 'files-btn files-btn-lg'

  return (
    <div ref={root} tabIndex={-1} onKeyDown={onKeyDown} data-testid="select-dialog" data-mode={mode}
      className={clsx('files-ui relative flex min-h-0 flex-col gap-3 text-[color:var(--cp-text)] outline-none', compact ? 'h-[min(70dvh,560px)]' : 'h-[min(60vh,520px)]')}>
      <div className="sr-only" role="status" aria-live="polite" data-testid="select-status">{status}</div>
      {localEntry && <input ref={localInput} type="file" hidden data-testid="select-local-input" multiple={input.multiple || mode === 'folder'} accept={mode === 'open' ? inputAccept(input.accept) : undefined}
        {...(mode === 'folder' ? { webkitdirectory: '' } : {})}
        onChange={(event) => { const files = Array.from(event.target.files ?? []); event.target.value = ''; pickedLocal(files) }} />}

      <div className={clsx('flex min-h-0 flex-1 gap-3', compact && 'flex-col')}>
        {compact ? (
          <select aria-label={t('sysdlg.select.locations', 'Locations')} data-testid="select-locations" value={activeLocation?.url ?? ''} onChange={(event) => { if (event.target.value) navigate(event.target.value) }}
            className="files-input !min-h-10 !rounded-[10px] !px-2">
            {!activeLocation && <option value="">{t('sysdlg.select.locations', 'Locations')}</option>}
            {locations.map((location) => <option key={location.key} value={location.url}>{location.label}</option>)}
          </select>
        ) : (
          <nav aria-label={t('sysdlg.select.locations', 'Locations')} className="desktop-scrollbar w-44 shrink-0 overflow-y-auto" data-testid="select-locations">
            {locations.map((location) => <button type="button" key={location.key} aria-current={activeLocation?.key === location.key ? 'location' : undefined} onClick={() => navigate(location.url)}
              className={clsx('flex h-8 w-full items-center gap-2 rounded-[8px] px-2 text-left text-[14px] transition-colors', activeLocation?.key === location.key ? 'bg-[color:var(--files-selected)] font-medium' : 'hover:bg-[color:var(--files-hover)]')}>{location.icon}<span className="truncate">{location.label}</span></button>)}
            {dfs.state.status === 'error' && <button type="button" className="files-btn files-btn-ghost mt-1" onClick={dfs.retry}>{t('sysdlg.select.retry', 'Retry')}</button>}
          </nav>
        )}

        <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-2">
          <div className="relative">
            <Search size={16} aria-hidden="true" className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-[color:var(--cp-muted)]" />
            <input type="search" value={query} data-testid="select-search" aria-label={t('sysdlg.select.search', 'Search files')} placeholder={t('sysdlg.select.search', 'Search files')}
              enterKeyHint="search" autoComplete="off" spellCheck={false} onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => { if (event.key === 'Escape' && query) { event.stopPropagation(); setQuery('') } }}
              className="files-input !min-h-10 !rounded-[10px] !pl-9 !pr-9 [&::-webkit-search-cancel-button]:hidden" />
            {query && <button type="button" data-testid="select-search-clear" aria-label={t('sysdlg.select.searchClear', 'Clear search')} onClick={() => setQuery('')}
              className="files-icon-btn absolute right-1 top-1/2 -translate-y-1/2"><X size={16} aria-hidden="true" /></button>}
          </div>
          {searching ? <p className="min-h-8 px-1.5 text-[13px] leading-8 text-[color:var(--cp-muted)]" data-testid="select-search-scope">{t('sysdlg.select.searchScope', 'Results from all your files')}</p> :
          <nav aria-label={t('sysdlg.select.path', 'Path')} className="flex flex-wrap items-center gap-0.5 text-[13px] text-[color:var(--cp-muted)]" data-testid="select-crumbs">
            {crumbsForUrl(url, url === RECENT_URL ? t('sysdlg.select.recent', 'Recent') : list.meta?.title).map((crumb, index, all) => <span key={crumb.url} className="flex items-center">
              {index > 0 && <ChevronRight size={14} aria-hidden="true" className="text-[color:var(--cp-muted)]" />}
              <button type="button" disabled={index === all.length - 1} aria-current={index === all.length - 1 ? 'page' : undefined} onClick={() => navigate(crumb.url)}
                className="min-h-8 max-w-40 truncate rounded-[6px] px-1.5 transition-colors hover:bg-[color:var(--files-hover)] hover:text-[color:var(--cp-text)] disabled:font-medium disabled:text-[color:var(--cp-text)] disabled:hover:bg-transparent">{crumb.label === 'root' && index === 0 ? '/' : crumb.label}</button>
            </span>)}
            {mode !== 'open' && path !== null && writable && <button type="button" data-testid="select-new-folder" onClick={() => setCreating(true)} className="files-btn files-btn-ghost ml-auto"><FolderPlus size={16} aria-hidden="true" />{t('sysdlg.select.newFolder', 'New folder')}</button>}
          </nav>}

          <div role="listbox" aria-label={t('sysdlg.select.items', 'Files and folders')} aria-multiselectable={input.multiple || undefined} aria-busy={loading} data-testid="select-list" data-layout={grid ? 'grid' : 'list'}
            className="desktop-scrollbar min-h-0 flex-1 overflow-y-auto rounded-[10px] border border-[color:var(--cp-border)]"
            onScroll={(event) => { const el = event.currentTarget; if (more && !loading && el.scrollTop + el.clientHeight > el.scrollHeight - 120) loadMore() }}>
            {!searching && list.status === 'error' ? (
              <div role="alert" className="flex flex-col items-start gap-3 p-4 text-[14px]" data-testid="select-error">
                <p>{signedOut ? t('sysdlg.select.signInHint', 'Your session has ended. Sign in again, then retry — your selection is kept.') : list.error?.message}</p>
                <div className="flex gap-2">
                  {signedOut && <a className="files-btn files-btn-primary" href={new URL('/login', window.location.origin).href} target="_blank" rel="noreferrer">{t('sysdlg.select.signIn', 'Sign in again')}</a>}
                  <button type="button" className="files-btn" onClick={() => list.reload()}>{t('sysdlg.select.retry', 'Retry')}</button>
                </div>
              </div>
            ) : searching && search.state.status === 'error' && !rows.length ? (
              <div role="alert" className="flex flex-col items-start gap-3 p-4 text-[14px]" data-testid="select-error">
                <p>{search.state.error?.fallback}</p>
                {search.state.error?.retryable !== false && <button type="button" className="files-btn" onClick={search.retry}>{t('sysdlg.select.retry', 'Retry')}</button>}
              </div>
            ) : <>
              <div className={grid ? 'grid grid-cols-[repeat(auto-fill,minmax(88px,1fr))] gap-1 p-2' : undefined}>
                {rows.map((item) => {
                  const { entry } = item
                  const folder = entry.kind === 'folder'
                  const isSelected = selected.has(entry.path)
                  const usable = folder ? !isExcluded(entry.path) : mode === 'save' || (selectable(entry) && (isSelected || !full))
                  const reason = usable ? undefined : folder || mode !== 'open' ? t('sysdlg.select.excluded', 'This folder cannot be the destination') : selectable(entry) ? t('sysdlg.select.limitReached', 'You can choose up to {{limit}} items', { limit: input.limit }) : t('sysdlg.select.notAccepted', 'This type cannot be chosen here')
                  const check = mode === 'open' && input.multiple && !folder && <span aria-hidden="true" className={clsx('flex size-4 shrink-0 items-center justify-center rounded-[4px] border', isSelected ? 'border-transparent bg-[color:var(--files-primary)] text-[color:var(--files-on-primary)]' : 'border-[color:color-mix(in_srgb,var(--cp-text)_28%,transparent)] bg-[color:var(--cp-surface-opaque)]')}>{isSelected && <Check size={12} />}</span>
                  const common = {
                    type: 'button' as const, role: 'option', 'aria-selected': isSelected, 'aria-disabled': !usable, title: reason ?? (searching ? entry.path : undefined),
                    'data-testid': 'select-row', 'data-name': entry.name, 'data-kind': folder ? 'folder' : 'file',
                    onClick: () => { clicks.current = [clicks.current[1], item.key]; if (usable) activate(item) },
                    onDoubleClick: () => { if (mode === 'open' && !input.multiple && selectable(entry) && clicks.current[0] === item.key) finish({ action: 'selected', items: [zoneContent(entry)] }) },
                  }
                  if (grid) return <button key={item.key} {...common}
                    className={clsx('relative flex min-w-0 flex-col gap-1.5 rounded-[10px] border border-transparent p-1.5 text-left text-[12px] transition-colors', isSelected ? 'border-[color:var(--files-selected-line)] bg-[color:var(--files-selected)]' : 'hover:bg-[color:var(--files-hover)]', !usable && 'cursor-not-allowed opacity-45')}>
                    <span className="flex aspect-square w-full items-center justify-center overflow-hidden rounded-[6px] border border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-2-opaque)]"><Thumb entry={entry} size={28} box={256} scope="all" /></span>
                    <span className="w-full truncate px-0.5">{entry.name}</span>
                    {check && <span className="absolute right-2 top-2">{check}</span>}
                  </button>
                  return <button key={item.key} {...common}
                    className={clsx('flex min-h-11 w-full items-center gap-3 border-b border-[color:var(--files-hairline)] px-3 text-left text-[14px] transition-colors',
                      isSelected ? 'bg-[color:var(--files-selected)]' : 'hover:bg-[color:var(--files-hover)]', !usable && 'cursor-not-allowed opacity-45', mode === 'save' && !folder && 'text-[color:var(--cp-muted)]')}>
                    {check}
                    <span className="flex size-7 shrink-0 items-center justify-center overflow-hidden rounded-md"><Thumb entry={entry} size={18} box={256} scope="media" /></span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate">{entry.name}</span>
                      {searching && <span className="block truncate text-[12px] text-[color:var(--cp-muted)]">{entry.path.slice(0, entry.path.lastIndexOf('/')) || '/'}</span>}
                    </span>
                    {!folder && entry.sizeBytes !== undefined && <span className="shrink-0 text-[12px] tabular-nums text-[color:var(--cp-muted)]">{formatBytes(entry.sizeBytes)}</span>}
                    {folder && <ChevronRight size={16} aria-hidden="true" className="shrink-0 text-[color:var(--cp-muted)]" />}
                  </button>
                })}
              </div>
              {loading && <p className="p-4 text-[13px] text-[color:var(--cp-muted)]">{searching ? t('sysdlg.select.searching', 'Searching…') : t('sysdlg.select.loading', 'Loading…')}</p>}
              {!loading && (searching ? search.state.status === 'success' : ready) && !rows.length && !more && <p className="p-4 text-[13px] text-[color:var(--cp-muted)]" data-testid="select-empty">
                {searching ? t('sysdlg.select.noMatches', 'No matching files') : mode === 'folder' ? t('sysdlg.select.emptyFolders', 'No folders here') : t('sysdlg.select.empty', 'Nothing here')}</p>}
              {!loading && more && <button type="button" data-testid="select-load-more" className="min-h-11 w-full text-[13px] text-[color:var(--cp-muted)] transition-colors hover:bg-[color:var(--files-hover)] hover:text-[color:var(--cp-text)]" onClick={loadMore}>{t('sysdlg.select.loadMore', 'Load more')}</button>}
            </>}
          </div>
        </div>
      </div>

      {mode !== 'open' && path !== null && ready && !writable && (mode === 'save' || input.writable) && <p role="alert" className="text-sm text-[color:var(--cp-danger)]" data-testid="select-readonly">{t('sysdlg.select.readOnly', 'You cannot save to this location. Choose another folder.')}</p>}
      {excluded && <p role="alert" className="text-sm text-[color:var(--cp-danger)]" data-testid="select-excluded">{t('sysdlg.select.excluded', 'This folder cannot be the destination')}</p>}
      {notice && <p role="alert" className="text-sm text-[color:var(--cp-danger)]" data-testid="select-notice">{notice}</p>}
      {conflict && <div role="alertdialog" aria-label={t('sysdlg.select.replace', 'Replace')} className="flex flex-wrap items-center gap-2 rounded-[10px] border border-[color:var(--cp-border)] p-3 text-[14px]" data-testid="select-conflict">
        <p className="min-w-0 flex-1">{t('sysdlg.select.exists', '“{{name}}” already exists here. Replace it?', { name: conflict })}</p>
        <button type="button" className={buttonClass} onClick={() => setConflict(null)}>{t('sysdlg.select.keep', 'Keep editing')}</button>
        <button type="button" autoFocus data-testid="select-replace" className={clsx(buttonClass, 'files-btn-danger')} onClick={() => void confirm(true)}>{t('sysdlg.select.replace', 'Replace')}</button>
      </div>}

      <form className="flex flex-wrap items-center gap-2" onSubmit={(event) => { event.preventDefault(); void confirm() }}>
        {mode === 'save' && <input aria-label={t('sysdlg.select.fileName', 'File name')} data-testid="select-name" value={name} maxLength={255} enterKeyHint="done" autoComplete="off" spellCheck={false}
          aria-invalid={!!name && !nameCheck.success} onChange={(event) => { setName(event.target.value); setConflict(null); setNotice(null) }}
          className="files-input !min-h-10 !w-auto flex-[1_1_12rem] !rounded-[10px]" placeholder={t('sysdlg.select.fileName', 'File name')} />}
        {mode === 'open' && <p className="min-w-0 flex-1 truncate text-[13px] tabular-nums text-[color:var(--cp-muted)]" data-testid="select-count">
          {full ? t('sysdlg.select.limitReached', 'You can choose up to {{limit}} items', { limit: input.limit }) : selected.size ? t('sysdlg.select.selectedCount', '{{count}} selected', { count: selected.size }) : ''}</p>}
        {mode === 'folder' && <span className="flex-1" />}
        {localEntry && <button type="button" data-testid="select-local" onClick={openLocal} className={buttonClass}>
          <MonitorSmartphone size={16} aria-hidden="true" />{t(`sysdlg.select.fromDevice.${mode}`)}</button>}
        <button type="submit" data-testid="select-confirm" disabled={!canConfirm || !!conflict} className={clsx(buttonClass, 'files-btn-primary')}>{confirmLabel}</button>
      </form>

      <NamePromptDialog request={creating && path !== null ? { title: t('sysdlg.select.newFolder', 'New folder'), label: t('sysdlg.select.folderName', 'Folder name'), submitLabel: t('sysdlg.select.create', 'Create'), schema: entryNameSchema,
        onSubmit: async (folderName) => { try { await folderOps().createFolder(path, folderName) } catch (error) { throw new Error(toUiError(error).fallback) } navigate(`${path === '/' ? '' : path}/${folderName}`) } } : null} onClose={() => setCreating(false)} />
    </div>
  )
}
