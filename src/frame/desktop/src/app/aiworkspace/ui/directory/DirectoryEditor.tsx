/* The standard editor of a directory (框架 PRD §7.1, §7.2, §7.4): one component in three sizes — a Block on a canvas or
 * a flow page (`embedded`), a tab of the data mode (`tab`), the phone's full screen (`full`). A breadcrumb on top (a
 * child directory or a folder of names opens in the same editor), the rows (picture or kind icon, name, size; folders
 * first) as a list or a thumbnail grid, "＋ Add" (from the Zone first, this device, a new subdirectory, an existing
 * directory), a ⋯ on every row, long press for several rows, the preview and the uploads. Everything works by tapping:
 * no hover, right click, keyboard or exact dragging is needed; on a desktop files can also be dropped in and rows
 * dragged onto a folder or a breadcrumb step. Writes go through ./ops (existing directory operations). */

import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react'
import { ArrowLeft, ChevronRight, Copy, Download, Eye, FolderDown, FolderInput, FolderOpen, FolderPlus, LayoutGrid, Link2, List, ListChecks, MoreHorizontal, Pencil, Plus, Replace, Trash2, Upload, X } from 'lucide-react'
import type { ReadOk } from '../../api/session'
import { describeError } from '../../api/session'
import { useEntity, useLoad, useOutlineVersion, useReadOnlyReason, useStore, useUserState, useVersion, useWriteAccess } from '../../state/hooks'
import { aiwsT } from '../../i18n'
import { useAiwsText } from '../../useAiwsText'
import { LockScope } from '../blocks/editors'
import { consumeIntent, useIntent } from '../blocks/editorToolbar'
import { anchorOf, type AnchorRect } from '../controls/data/ControlPopover'
import { copyRecordLink, useRecordFocus } from '../controls/data/focus'
import { saveBlob } from '../shell/download'
import { useDirectorySaveToZone } from './saveToZone'
import { useFilePicker } from '../shell/filePicker'
import { usePhone } from '../shell/shellContext'
import { entityLabel } from '../sources/dataOps'
import { formatBytes } from './format'
import { entriesAt, keepBothName, nameTaken, pathOf, placeInside, placeOf, resolvePath, samePlace, type Crumb, type DirectoryContent, type Entry, type Place } from './model'
import { addExistingDirectory, createSubdirectory, moveConflicts, moveEntries, nameError, readDirectory, removeEntries, renameEntry, type Resolution } from './ops'
import { ConfirmDialog, ConflictDialog, DirDialog, DirMenu, ExistingDialog, MoveDialog, NameDialog, PreviewPane, Thumb, UploadList, type DirMenuItem, type PreviewFile } from './parts'
import { uploadQueue } from './uploads'
import './directory.css'

export interface DirectoryEditorProps {
  /** The directory (`buckyos.dir`) at the root of the breadcrumb. */
  entityId: string
  /** `embedded`: inside a Block (canvas, flow page); `tab`: a data mode tab; `full`: a phone's full screen. */
  size: 'embedded' | 'tab' | 'full'
  /** `edit`: the editor (writes when allowed); `browse`: open folders and preview, no writes (view mode); `static`: a
   * compact listing that takes no input (a canvas Block that is not being edited). Default `edit`. */
  variant?: 'edit' | 'browse' | 'static'
  /** Why writes are refused here (mode, capability, offline…), or null. The directory's own capabilities, source and
   * lock are checked as well. */
  readOnlyReason?: string | null
  /** The Block showing the directory: a member's link names it, and a link to a member opens it here. */
  cellId?: string
  /** `full`: the back button of the top bar. */
  onClose?: () => void
}

type MenuState = { kind: 'add' | 'more'; anchor: AnchorRect } | { kind: 'row'; anchor: AnchorRect; entry: Entry }
type DialogState =
  | { kind: 'rename'; entry: Entry }
  | { kind: 'mkdir' }
  | { kind: 'existing' }
  | { kind: 'remove'; entries: Entry[] }
  | { kind: 'move'; entries: Entry[] }
  | { kind: 'conflict'; names: string[]; done: (resolutions: Record<string, Resolution> | null) => void }
  | { kind: 'linkless' }

const MOVE_MIME = 'application/x-aiws-directory-rows'
/** Tells a preview opened again from the list from the one already open (its key remounts it). */
let previewNonce = 0
const nextPreviewNonce = () => { previewNonce += 1; return previewNonce }
/** A link to a member waiting for its Block's editor (Block id → path below the directory). */
const linkRequests = new Map<string, string>()
const LONG_PRESS_MS = 500

export function DirectoryEditor({ entityId: rootId, size, variant = 'edit', readOnlyReason = null, cellId, onClose }: DirectoryEditorProps) {
  const { t, compare } = useAiwsText()
  const store = useStore()
  const phone = usePhone()
  useOutlineVersion()
  const root = useEntity(rootId)
  const [crumbs, setCrumbs] = useState<Crumb[]>([])
  const place = placeOf(rootId, crumbs)
  const current = useEntity(place.entityId)
  const version = useVersion(`e:${place.entityId}`)
  const load = useCallback(() => store.session.read<DirectoryContent>(place.entityId), [store, place.entityId])
  const read = useLoad<ReadOk<DirectoryContent>>(load, version)
  const data = read.data?.entity_id === place.entityId ? read.data : undefined
  const members = useMemo(() => data?.content.payload.members ?? {}, [data])
  const entries = useMemo(() => entriesAt(members, place.prefix, compare), [members, place.prefix, compare])
  const files: PreviewFile[] = useMemo(() => entries.flatMap((entry) => (entry.kind === 'file' ? [{ key: entry.key, name: entry.name, member: entry.member }] : [])), [entries])
  const viewPref = useUserState<string>('ui:directory-view')
  const view: 'list' | 'grid' = viewPref === 'grid' ? 'grid' : 'list'

  const windowReadOnly = useReadOnlyReason()
  const access = useWriteAccess(current)
  const capable = Boolean(current && !current.source && !current.degraded && current.capabilities.includes('update'))
  const mayEdit = variant === 'edit' && !readOnlyReason && !windowReadOnly && capable
  const canWrite = mayEdit && access.editable && Boolean(data) && !read.error
  const canExport = Boolean(current?.capabilities.includes('export'))
  const readOnlyText = variant !== 'edit' ? null : readOnlyReason ?? windowReadOnly ?? (current?.source ? t('aiworkspace.directory.readOnlySource') : current && !capable ? t('aiworkspace.directory.readOnly') : null)

  const [menu, setMenu] = useState<MenuState | null>(null)
  const [dialog, setDialog] = useState<DialogState | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [selecting, setSelecting] = useState(false)
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set())
  const [preview, setPreview] = useState<{ key: string; nonce: number } | null>(null)
  const [shownKey, setShownKey] = useState<string | null>(null)
  const [dropping, setDropping] = useState(false)
  const addButton = useRef<HTMLButtonElement>(null)
  const localInput = useRef<HTMLInputElement>(null)
  const picker = useFilePicker(setError)
  const queue = uploadQueue(store)

  const placeKey = `${place.entityId}:${place.prefix}`
  // a new place starts without a selection or a preview
  // (a link to a member asks for its preview together with the move to its folder)
  const [seenPlace, setSeenPlace] = useState(placeKey)
  const [pendingPreview, setPendingPreview] = useState<string | null>(null)
  if (seenPlace !== placeKey) {
    setSeenPlace(placeKey); setSelecting(false); setSelected(new Set()); setMenu(null)
    setPreview(pendingPreview ? { key: pendingPreview, nonce: 0 } : null); setShownKey(pendingPreview); setPendingPreview(null)
  } else if (pendingPreview !== null) setPendingPreview(null)
  // a folder of names that is gone (its last member moved or renamed elsewhere): up one level
  if (data && place.prefix && crumbs.at(-1)?.kind === 'folder' && !Object.keys(members).some((key) => key.startsWith(place.prefix))) setCrumbs(crumbs.slice(0, -1))

  const enter = (entry: Entry) => {
    if (!placeInside(place, entry)) return
    setCrumbs([...crumbs, entry.kind === 'dir' ? { kind: 'dir', name: entry.name, entityId: entry.member.entity_id } : { kind: 'folder', name: entry.name }])
  }
  const openPreview = (key: string) => { setPreview({ key, nonce: nextPreviewNonce() }); setShownKey(key) }

  // a link to a member (`record=<path>`) opens its folder and previews it
  // (kept until an editor of that Block applies it: the first one mounted may be replaced before the path resolves)
  const [linkTick, setLinkTick] = useState(0)
  useRecordFocus(cellId ?? '', (focus) => { if (cellId) { linkRequests.set(cellId, focus.record); setLinkTick((n) => n + 1) } })
  useEffect(() => {
    const path = cellId ? linkRequests.get(cellId) : undefined
    if (!cellId || path === undefined) return
    let live = true
    void resolvePath(rootId, path, async (id) => (await store.session.read<DirectoryContent>(id).catch(() => null))?.content.payload.members).then((found) => {
      if (!live) return
      linkRequests.delete(cellId)
      if (!found) { store.notify('info', aiwsT('aiworkspace.directory.linkMissing')); return }
      const target = placeOf(rootId, found.crumbs)
      const key = found.name !== null && variant !== 'static' ? `${target.prefix}${found.name}` : null
      setPendingPreview(key)
      setCrumbs(found.crumbs)
      // already there: the place does not change, the preview opens now
      if (key) { setPreview({ key, nonce: nextPreviewNonce() }); setShownKey(key) }
    })
    return () => { live = false }
  }, [cellId, linkTick, rootId, store, variant])

  const run = async (action: () => Promise<unknown>) => {
    setError(null)
    try { await action() } catch (failure) { setError(describeError(failure)) }
  }

  /** "Same name here": the user's choice for each, or null when they cancelled. */
  const askConflicts = (names: string[]) => new Promise<Record<string, Resolution> | null>((resolve) => {
    setDialog({ kind: 'conflict', names, done: (resolutions) => { setDialog(null); resolve(resolutions) } })
  })

  /** Files into the current place: a name already here asks (replace, keep both, skip); then each uploads on its own. */
  const addFiles = (chosen: File[]) => run(async () => {
    if (!canWrite || chosen.length === 0) return
    const target = place
    const fresh = (await readDirectory(store, target.entityId)).content.payload.members ?? {}
    const batch = new Set<string>()
    const conflicts: string[] = []
    for (const file of chosen) {
      if ((nameTaken(fresh, target.prefix, file.name) || batch.has(file.name)) && !conflicts.includes(file.name)) conflicts.push(file.name)
      batch.add(file.name)
    }
    const resolutions = conflicts.length > 0 ? await askConflicts(conflicts) : {}
    if (!resolutions) return
    const used = new Set<string>()
    const taken = (name: string) => nameTaken(fresh, target.prefix, name) || used.has(name)
    const items: { file: File; name: string; mode: 'new' | 'replace' }[] = []
    const invalid: string[] = []
    for (const file of chosen) {
      const problem = nameError({}, target.prefix, file.name)
      if (problem) { invalid.push(file.name); continue }
      const resolution = taken(file.name) ? resolutions[file.name] ?? 'skip' : null
      if (resolution === 'skip') continue
      // a folder or a subdirectory of that name is never written over by a file
      const replaceable = resolution === 'replace' && fresh[`${target.prefix}${file.name}`]?.kind === 'file' && !used.has(file.name)
      const name = resolution === 'keep' || (resolution === 'replace' && !replaceable) ? keepBothName(file.name, taken) : file.name
      used.add(name)
      items.push({ file, name, mode: replaceable ? 'replace' : 'new' })
    }
    if (invalid.length) setError(t('aiworkspace.directory.name.invalidFiles', { names: invalid.join(', ') }))
    queue.add(rootId, target, items)
  })

  const freeName = (base: string) => (nameTaken(members, place.prefix, base) ? keepBothName(base, (n) => nameTaken(members, place.prefix, n)) : base)
  const pickFromZone = () => { void picker.files({ multiple: true, title: t('aiworkspace.directory.add.zoneTitle') }).then((chosen) => { if (chosen?.length) void addFiles(chosen) }) }
  const pickFromDevice = () => localInput.current?.click()

  const doMove = (moving: Entry[], to: Place) => run(async () => {
    if (samePlace(to, place)) return
    const names = await moveConflicts(store, moving, to)
    const resolutions = names.length > 0 ? await askConflicts(names) : {}
    if (!resolutions) return
    await moveEntries(store, place, moving, to, resolutions)
    setSelecting(false)
    setSelected(new Set())
  })

  const download = (targets: Entry[]) => run(async () => {
    for (const entry of targets) if (entry.kind === 'file') saveBlob(await store.session.fetchAsset(entry.member.object_id), entry.name)
  })
  // keeping a file is saving it into the Zone (the user's main file system); downloading stays the other entry
  const saveZone = useDirectorySaveToZone(store)
  const saveToZone = (targets: Entry[]) => run(async () => {
    const saved = await saveZone(targets.flatMap((entry) => (entry.kind === 'file' ? [{ name: entry.name, objectId: entry.member.object_id }] : [])))
    if (saved.status !== 'saved') return
    store.notify('info', saved.count === 1 ? t('aiworkspace.directory.saveToZone.saved', { path: saved.path }) : t('aiworkspace.directory.saveToZone.savedMany', { count: saved.count, path: saved.path }))
    if (selecting) { setSelecting(false); setSelected(new Set()) }
  })

  const linkCell = cellId ?? store.outline.all().find((e) => e.type_id === 'buckyos.cell' && e.source_id === rootId && !e.deleted)?.entity_id
  const copyLink = (entry: Entry) => {
    if (!linkCell) { setDialog({ kind: 'linkless' }); return }
    copyRecordLink(store, linkCell, pathOf(crumbs, entry.name), null, t('aiworkspace.directory.linkCopied'))
  }

  const replaceFile = (entry: Entry) => {
    if (entry.kind !== 'file') return
    void picker.file().then((file) => { if (file) queue.add(rootId, place, [{ file, name: entry.name, mode: 'replace' }]) })
  }

  // the near toolbar's "add" on a canvas: the editor opens with the add menu
  const intent = useIntent(cellId ?? '')
  useEffect(() => {
    // it waits for the directory: "add" needs to know whether this editor may write
    if (!intent || !cellId || variant !== 'edit' || !data) return
    consumeIntent(cellId, intent)
    if (intent.value === 'add' && canWrite) setMenu({ kind: 'add', anchor: anchorOf(addButton.current) })
  }, [intent, cellId, variant, canWrite, data])

  const toggle = (entry: Entry) => setSelected((previous) => {
    const next = new Set(previous)
    if (next.has(entry.id)) next.delete(entry.id)
    else next.add(entry.id)
    return next
  })
  const chosenEntries = entries.filter((entry) => selected.has(entry.id))
  const startSelecting = (entry?: Entry) => { setSelecting(true); setSelected(new Set(entry ? [entry.id] : [])); setPreview(null) }

  const addItems: DirMenuItem[] = [
    { id: 'zone', label: t('aiworkspace.directory.add.zone'), icon: FolderInput, testId: 'aiws-directory-add-zone', run: pickFromZone },
    { id: 'device', label: t('aiworkspace.directory.add.device'), icon: Upload, testId: 'aiws-directory-add-device', run: pickFromDevice },
    { id: 'mkdir', label: t('aiworkspace.directory.add.folder'), icon: FolderPlus, testId: 'aiws-directory-add-folder', run: () => setDialog({ kind: 'mkdir' }) },
    { id: 'existing', label: t('aiworkspace.directory.add.existing'), icon: FolderOpen, testId: 'aiws-directory-add-existing', run: () => setDialog({ kind: 'existing' }) },
  ]
  const rowItems = (entry: Entry): DirMenuItem[] => [
    ...(entry.kind === 'file' ? [{ id: 'preview', label: t('aiworkspace.directory.preview'), icon: Eye, testId: 'aiws-directory-action-preview', run: () => openPreview(entry.key) }]
      : [{ id: 'open', label: t('aiworkspace.directory.open'), icon: FolderOpen, testId: 'aiws-directory-action-open', run: () => enter(entry) }]),
    ...(mayEdit ? [{ id: 'rename', label: t('aiworkspace.directory.rename'), icon: Pencil, testId: 'aiws-directory-rename', disabled: canWrite ? false as const : t('aiworkspace.directory.working'), run: () => setDialog({ kind: 'rename', entry }) }] : []),
    ...(mayEdit && entry.kind === 'file' ? [{ id: 'replace', label: t('aiworkspace.directory.replace'), icon: Replace, testId: 'aiws-directory-replace', disabled: canWrite ? false as const : t('aiworkspace.directory.working'), run: () => replaceFile(entry) }] : []),
    ...(mayEdit ? [{ id: 'move', label: t('aiworkspace.directory.moveTo'), icon: FolderInput, testId: 'aiws-directory-move', disabled: canWrite ? false as const : t('aiworkspace.directory.working'), run: () => setDialog({ kind: 'move', entries: [entry] }) }] : []),
    ...(entry.kind === 'file' ? [
      { id: 'save-zone', label: t('aiworkspace.directory.saveToZone'), icon: FolderDown, testId: 'aiws-directory-save-zone', disabled: canExport ? false as const : t('aiworkspace.directory.needsExport'), run: () => { void saveToZone([entry]) } },
      { id: 'download', label: t('aiworkspace.directory.download'), icon: Download, testId: 'aiws-directory-download', disabled: canExport ? false as const : t('aiworkspace.directory.needsExport'), run: () => { void download([entry]) } },
    ] : []),
    { id: 'link', label: t('aiworkspace.directory.copyLink'), icon: Link2, testId: 'aiws-directory-copy-link', run: () => copyLink(entry) },
    ...(variant !== 'static' ? [{ id: 'select', label: t('aiworkspace.directory.select'), icon: ListChecks, testId: 'aiws-directory-select', run: () => startSelecting(entry) }] : []),
    ...(mayEdit ? [{ id: 'remove', label: t('aiworkspace.directory.remove'), icon: Trash2, danger: true, testId: 'aiws-directory-remove', disabled: canWrite ? false as const : t('aiworkspace.directory.working'), run: () => setDialog({ kind: 'remove', entries: [entry] }) }] : []),
  ]
  const moreItems: DirMenuItem[] = [
    { id: 'select', label: t('aiworkspace.directory.select'), icon: ListChecks, testId: 'aiws-directory-select-mode', disabled: entries.length ? false : t('aiworkspace.directory.emptyTitle'), run: () => startSelecting() },
    { id: 'view', label: t(view === 'grid' ? 'aiworkspace.directory.viewList' : 'aiworkspace.directory.viewGrid'), icon: view === 'grid' ? List : LayoutGrid, testId: 'aiws-directory-view-toggle', run: () => store.userState.set('ui:directory-view', view === 'grid' ? 'list' : 'grid') },
    ...(cellId || linkCell ? [{ id: 'link', label: t('aiworkspace.directory.copyFolderLink'), icon: Copy, testId: 'aiws-directory-copy-folder-link', run: () => { if (linkCell) copyRecordLink(store, linkCell, crumbs.map((c) => c.name).join('/'), null, t('aiworkspace.directory.linkCopied')) } }] : []),
  ]

  // ---- rows: tap opens (or toggles while selecting), ⋯ has the actions, a long press starts selecting
  const press = useRef<{ timer: number; x: number; y: number; fired: boolean; touch: boolean } | null>(null)
  const cancelPress = () => { if (press.current) window.clearTimeout(press.current.timer) }
  // the click a browser may send when the finger lifts after a long press is not a tap (it lands wherever selecting
  // moved things): it is dropped, until the next press
  const ghost = useRef(false)
  const swallowGhostClick = (event: ReactMouseEvent) => { if (ghost.current) { ghost.current = false; event.preventDefault(); event.stopPropagation() } }
  const rowHandlers = (entry: Entry) => variant === 'static' ? {} : {
    onPointerDown: (event: ReactPointerEvent) => {
      cancelPress()
      if (event.button !== 0) { press.current = null; return }
      const state = { timer: 0, x: event.clientX, y: event.clientY, fired: false, touch: event.pointerType !== 'mouse' }
      state.timer = window.setTimeout(() => {
        state.fired = true
        ghost.current = true
        if (selecting) toggle(entry)
        else startSelecting(entry)
      }, LONG_PRESS_MS)
      press.current = state
    },
    onPointerMove: (event: ReactPointerEvent) => { const p = press.current; if (p && Math.hypot(event.clientX - p.x, event.clientY - p.y) > 10) cancelPress() },
    onPointerUp: cancelPress,
    onPointerCancel: cancelPress,
    onContextMenu: (event: ReactMouseEvent) => {
      event.preventDefault()
      // a finger's long press selects; a mouse's right click opens the row's menu
      if (press.current?.touch) return
      setMenu({ kind: 'row', anchor: { left: event.clientX, top: event.clientY, width: 0, height: 0 }, entry })
    },
  }
  const activate = (entry: Entry) => {
    if (press.current?.fired) { press.current = null; return }
    press.current = null
    if (selecting) { toggle(entry); return }
    if (entry.kind === 'file') openPreview(entry.key)
    else enter(entry)
  }

  // ---- desktop: files dropped in are added; rows dragged onto a folder or a breadcrumb step move
  const draggable = canWrite && !phone && variant === 'edit'
  const dragRows = (event: DragEvent, entry: Entry) => {
    const ids = selecting && selected.has(entry.id) ? [...selected] : [entry.id]
    event.dataTransfer.setData(MOVE_MIME, JSON.stringify({ place, ids }))
    event.dataTransfer.effectAllowed = 'move'
  }
  const dropTarget = (to: Place) => ({
    onDragOver: (event: DragEvent) => { if (draggable && event.dataTransfer.types.includes(MOVE_MIME)) { event.preventDefault(); event.stopPropagation(); event.dataTransfer.dropEffect = 'move' } },
    onDrop: (event: DragEvent) => {
      const raw = event.dataTransfer.getData(MOVE_MIME)
      if (!raw) return
      event.preventDefault()
      event.stopPropagation()
      const moved = JSON.parse(raw) as { place: Place; ids: string[] }
      if (!samePlace(moved.place, place)) return
      void doMove(entries.filter((entry) => moved.ids.includes(entry.id)), to)
    },
  })
  const fileDrop = draggable ? {
    onDragOver: (event: DragEvent) => { if (event.dataTransfer.types.includes('Files')) { event.preventDefault(); setDropping(true) } },
    onDragLeave: (event: DragEvent) => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDropping(false) },
    onDrop: (event: DragEvent) => {
      if (!event.dataTransfer.types.includes('Files')) return
      event.preventDefault()
      setDropping(false)
      void addFiles([...event.dataTransfer.files])
    },
  } : {}

  const rootName = root ? entityLabel(root) : t('aiworkspace.directory.defaultName')
  const here = crumbs.at(-1)?.name ?? rootName
  const crumbPlace = (depth: number) => placeOf(rootId, crumbs.slice(0, depth))
  const breadcrumb = (
    <nav className="aiws-dir-crumbs" aria-label={t('aiworkspace.directory.crumbs')} data-testid="aiws-directory-crumbs">
      {crumbs.length > 0 && variant !== 'static' && (
        <button type="button" className="aiws-dir-icon" data-testid="aiws-directory-up" aria-label={t('aiworkspace.directory.up')} title={t('aiworkspace.directory.up')} onClick={() => setCrumbs(crumbs.slice(0, -1))}><ArrowLeft size={18} aria-hidden="true" /></button>
      )}
      {[rootName, ...crumbs.map((crumb) => crumb.name)].map((name, depth) => (
        <span key={depth} className="aiws-dir-crumb-step">
          {depth > 0 && <ChevronRight size={14} aria-hidden="true" />}
          {variant === 'static' ? <span className="aiws-dir-crumb">{name}</span> : (
            <button type="button" className="aiws-dir-crumb" data-testid="aiws-directory-crumb" data-depth={depth} aria-current={depth === crumbs.length ? 'location' : undefined}
              onClick={() => setCrumbs(crumbs.slice(0, depth))} {...(depth < crumbs.length ? dropTarget(crumbPlace(depth)) : {})}>{name}</button>
          )}
        </span>
      ))}
    </nav>
  )

  const metaOf = (entry: Entry) => entry.kind === 'file' ? (typeof entry.member.size === 'number' ? formatBytes(entry.member.size) : '')
    : entry.kind === 'folder' ? t('aiworkspace.directory.items', { count: entry.count }) : t('aiworkspace.directory.subdirectory')

  const row = (entry: Entry) => {
    const isSelected = selected.has(entry.id)
    const label = entry.kind === 'file' ? entry.name : t('aiworkspace.directory.folderNamed', { name: entry.name })
    if (variant === 'static') {
      return (
        <div key={entry.id} className="aiws-dir-row" role="listitem" data-testid="aiws-directory-member" data-name={entry.name} data-kind={entry.kind === 'file' ? 'file' : 'dir'}>
          <span className="aiws-dir-main"><Thumb kind={entry.kind} member={entry.kind === 'file' ? entry.member : undefined} name={entry.name} /><span className="aiws-dir-name">{entry.name}</span><span className="aiws-dir-meta">{metaOf(entry)}</span></span>
        </div>
      )
    }
    return (
      <div key={entry.id} className={`aiws-dir-row${isSelected ? ' is-selected' : ''}${shownKey !== null && preview && entry.kind === 'file' && entry.key === shownKey ? ' is-shown' : ''}`} role="listitem"
        data-testid="aiws-directory-member" data-name={entry.name} data-kind={entry.kind === 'file' ? 'file' : 'dir'} data-folder={entry.kind === 'folder' ? 'true' : undefined} data-selected={selecting ? String(isSelected) : undefined}
        draggable={draggable} onDragStart={draggable ? (event) => dragRows(event, entry) : undefined} {...(entry.kind !== 'file' ? dropTarget(placeInside(place, entry)!) : {})}>
        <button type="button" className="aiws-dir-main" data-testid="aiws-directory-open" aria-label={selecting ? t(isSelected ? 'aiworkspace.directory.deselectNamed' : 'aiworkspace.directory.selectNamed', { name: entry.name }) : label}
          aria-pressed={selecting ? isSelected : undefined} onClick={() => activate(entry)} {...rowHandlers(entry)}>
          {selecting && <span className={`aiws-dir-tick${isSelected ? ' is-on' : ''}`} aria-hidden="true" />}
          <Thumb kind={entry.kind} member={entry.kind === 'file' ? entry.member : undefined} name={entry.name} />
          <span className="aiws-dir-name">{entry.name}</span>
          <span className="aiws-dir-meta">{metaOf(entry)}</span>
        </button>
        {!selecting && (
          <button type="button" className="aiws-dir-icon aiws-dir-row-more" data-testid="aiws-directory-row-menu" aria-label={t('aiworkspace.directory.rowMenu', { name: entry.name })} title={t('aiworkspace.directory.rowMenu', { name: entry.name })}
            aria-haspopup="menu" onClick={(event) => setMenu({ kind: 'row', anchor: anchorOf(event.currentTarget), entry })}><MoreHorizontal size={18} aria-hidden="true" /></button>
        )}
      </div>
    )
  }

  const totalBytes = entries.reduce((sum, entry) => sum + (entry.kind === 'file' ? entry.member.size ?? 0 : entry.kind === 'folder' ? entry.size : 0), 0)
  const unreadable = read.error && !data
  const previewing = preview && variant !== 'static' && files.some((file) => file.key === preview.key)

  let body: ReactNode
  if (unreadable) body = (
    <div className="aiws-error" role="alert" data-testid="aiws-directory-error">{t('aiworkspace.directory.unreadable', { error: read.error ?? '' })}
      <button type="button" onClick={read.reload}>{t('aiworkspace.common.retry')}</button>
      {crumbs.length > 0 && <button type="button" onClick={() => setCrumbs(crumbs.slice(0, -1))}>{t('aiworkspace.directory.up')}</button>}
    </div>
  )
  else if (!data) body = <div className="aiws-muted aiws-dir-loading">{t('aiworkspace.directory.loading')}</div>
  else if (entries.length === 0) body = (
    <div className="aiws-dir-empty" data-testid="aiws-directory-empty">
      <FolderOpen size={32} aria-hidden="true" />
      <b>{t('aiworkspace.directory.emptyTitle')}</b>
      {canWrite ? (
        <>
          <span className="aiws-muted">{t('aiworkspace.directory.emptyHint')}</span>
          <div className="aiws-dir-empty-actions">
            <button type="button" className="is-primary" data-testid="aiws-directory-empty-zone" onClick={pickFromZone}><FolderInput size={18} aria-hidden="true" />{t('aiworkspace.directory.add.zone')}</button>
            <button type="button" data-testid="aiws-directory-empty-device" onClick={pickFromDevice}><Upload size={18} aria-hidden="true" />{t('aiworkspace.directory.add.device')}</button>
            <button type="button" data-testid="aiws-directory-empty-folder" onClick={() => setDialog({ kind: 'mkdir' })}><FolderPlus size={18} aria-hidden="true" />{t('aiworkspace.directory.add.folder')}</button>
          </div>
          <span className="aiws-muted aiws-dir-limits">{t('aiworkspace.directory.limits')}</span>
        </>
      ) : variant === 'edit' && <span className="aiws-muted">{t('aiworkspace.directory.empty')}</span>}
    </div>
  )
  else body = <div className={`aiws-dir-list is-${view}`} role="list" aria-label={here}>{entries.map(row)}</div>

  return (
    <LockScope entity={mayEdit ? current : undefined}>
      <section className={`aiws-directory aiws-dir is-${size} is-${variant}${dropping ? ' is-dropping' : ''}${phone ? ' is-phone' : ''}`} data-testid="aiws-directory" data-size={size} data-variant={variant}
        data-view={view} data-place={placeKey} aria-label={rootName} data-role={variant === 'edit' && size === 'embedded' ? 'editor' : undefined} onClickCapture={swallowGhostClick} onPointerDownCapture={() => { ghost.current = false }} {...fileDrop}>
        {size === 'full' && (
          <div className="aiws-dir-topbar">
            {onClose && <button type="button" className="aiws-dir-icon" data-testid="aiws-directory-close" aria-label={t('aiworkspace.directory.back')} title={t('aiworkspace.directory.back')} onClick={onClose}><ArrowLeft size={20} aria-hidden="true" /></button>}
            <b className="aiws-dir-title">{rootName}</b>
          </div>
        )}
        {variant !== 'static' && (
          <div className="aiws-dir-head">
            {size !== 'embedded' && <FolderOpen className="aiws-dir-head-icon" size={18} aria-hidden="true" />}
            {breadcrumb}
            {mayEdit && <button ref={addButton} type="button" className="aiws-dir-add is-primary" data-testid="aiws-directory-upload" disabled={!canWrite} aria-haspopup="menu"
              onClick={(event) => setMenu({ kind: 'add', anchor: anchorOf(event.currentTarget) })}><Plus size={18} aria-hidden="true" /><span>{t('aiworkspace.directory.add')}</span></button>}
            <button type="button" className="aiws-dir-icon" data-testid="aiws-directory-view" aria-label={t(view === 'grid' ? 'aiworkspace.directory.viewList' : 'aiworkspace.directory.viewGrid')} title={t(view === 'grid' ? 'aiworkspace.directory.viewList' : 'aiworkspace.directory.viewGrid')}
              onClick={() => store.userState.set('ui:directory-view', view === 'grid' ? 'list' : 'grid')}>{view === 'grid' ? <List size={18} aria-hidden="true" /> : <LayoutGrid size={18} aria-hidden="true" />}</button>
            <button type="button" className="aiws-dir-icon" data-testid="aiws-directory-more" aria-label={t('aiworkspace.directory.more')} title={t('aiworkspace.directory.more')} aria-haspopup="menu"
              onClick={(event) => setMenu({ kind: 'more', anchor: anchorOf(event.currentTarget) })}><MoreHorizontal size={18} aria-hidden="true" /></button>
          </div>
        )}
        {variant === 'static' && <div className="aiws-dir-head">{breadcrumb}</div>}
        {variant === 'edit' && readOnlyText && <div className="aiws-muted aiws-dir-note" data-testid="aiws-directory-readonly">{readOnlyText}</div>}
        {error && <div className="aiws-error" role="alert" data-testid="aiws-directory-error">{error} <button type="button" className="aiws-dir-icon" aria-label={t('aiworkspace.common.close')} onClick={() => setError(null)}><X size={14} aria-hidden="true" /></button></div>}
        {variant !== 'static' && <UploadList rootId={rootId} />}
        <div className={`aiws-dir-body${previewing ? ' has-preview' : ''}`}>
          {body}
          {previewing && preview && <PreviewPane key={`${placeKey}:${preview.key}:${preview.nonce}`} files={files} start={preview.key} allowExport={canExport} onShown={setShownKey} onClose={() => { setPreview(null); setShownKey(null) }} />}
        </div>
        {dropping && <div className="aiws-dir-dropping" aria-hidden="true">{t('aiworkspace.directory.dropHere')}</div>}
        {selecting ? (
          <div className="aiws-dir-bar" role="toolbar" aria-label={t('aiworkspace.directory.selected', { count: selected.size })} data-testid="aiws-directory-selection">
            <button type="button" className="aiws-dir-icon" data-testid="aiws-directory-bulk-cancel" aria-label={t('aiworkspace.common.cancel')} title={t('aiworkspace.common.cancel')} onClick={() => { setSelecting(false); setSelected(new Set()) }}><X size={18} aria-hidden="true" /></button>
            <span className="aiws-dir-bar-count">{t('aiworkspace.directory.selected', { count: selected.size })}</span>
            <button type="button" data-testid="aiws-directory-bulk-all" onClick={() => setSelected(selected.size === entries.length ? new Set() : new Set(entries.map((e) => e.id)))}>{t(selected.size === entries.length ? 'aiworkspace.directory.selectNone' : 'aiworkspace.directory.selectAll')}</button>
            <span className="aiws-grow" />
            {mayEdit && <button type="button" data-testid="aiws-directory-bulk-move" aria-label={t('aiworkspace.directory.move.short')} title={t('aiworkspace.directory.move.short')} disabled={!canWrite || selected.size === 0} onClick={() => setDialog({ kind: 'move', entries: chosenEntries })}><FolderInput size={18} aria-hidden="true" /><span>{t('aiworkspace.directory.move.short')}</span></button>}
            <button type="button" data-testid="aiws-directory-bulk-save-zone" disabled={selected.size === 0 || !canExport || chosenEntries.some((e) => e.kind !== 'file')}
              aria-label={t('aiworkspace.directory.saveToZone')} title={!canExport ? t('aiworkspace.directory.needsExport') : chosenEntries.some((e) => e.kind !== 'file') ? t('aiworkspace.directory.saveToZone.filesOnly') : t('aiworkspace.directory.saveToZone')}
              onClick={() => { void saveToZone(chosenEntries) }}><FolderDown size={18} aria-hidden="true" /><span>{t('aiworkspace.directory.saveToZone')}</span></button>
            <button type="button" data-testid="aiws-directory-bulk-download" disabled={selected.size === 0 || !canExport || chosenEntries.some((e) => e.kind !== 'file')}
              aria-label={t('aiworkspace.directory.download')} title={!canExport ? t('aiworkspace.directory.needsExport') : chosenEntries.some((e) => e.kind !== 'file') ? t('aiworkspace.directory.filesOnly') : t('aiworkspace.directory.download')}
              onClick={() => { void download(chosenEntries) }}><Download size={18} aria-hidden="true" /><span>{t('aiworkspace.directory.download')}</span></button>
            {mayEdit && <button type="button" className="is-danger" data-testid="aiws-directory-bulk-remove" aria-label={t('aiworkspace.directory.remove')} title={t('aiworkspace.directory.remove')} disabled={!canWrite || selected.size === 0} onClick={() => setDialog({ kind: 'remove', entries: chosenEntries })}><Trash2 size={18} aria-hidden="true" /><span>{t('aiworkspace.directory.remove')}</span></button>}
          </div>
        ) : data && entries.length > 0 && (
          <div className="aiws-dir-foot" data-testid="aiws-directory-count">
            <span>{t('aiworkspace.directory.items', { count: entries.length })}{totalBytes > 0 ? ` · ${formatBytes(totalBytes)}` : ''}</span>
            {size !== 'embedded' && variant === 'edit' && phone && mayEdit && <button type="button" className="aiws-dir-add" data-testid="aiws-directory-foot-add" disabled={!canWrite} onClick={(event) => setMenu({ kind: 'add', anchor: anchorOf(event.currentTarget) })}><Plus size={18} aria-hidden="true" />{t('aiworkspace.directory.add')}</button>}
          </div>
        )}
        {mayEdit && <input ref={localInput} type="file" multiple hidden data-testid="aiws-directory-local-input" onChange={(event) => {
          const chosen = [...(event.target.files ?? [])]
          event.target.value = ''
          if (chosen.length) void addFiles(chosen)
        }} />}

        {menu?.kind === 'add' && <DirMenu anchor={menu.anchor} label={t('aiworkspace.directory.add.title')} items={addItems} onClose={() => setMenu(null)} testId="aiws-directory-add-menu" />}
        {menu?.kind === 'more' && <DirMenu anchor={menu.anchor} label={t('aiworkspace.directory.more')} items={moreItems} onClose={() => setMenu(null)} testId="aiws-directory-more-menu" />}
        {menu?.kind === 'row' && <DirMenu anchor={menu.anchor} label={menu.entry.name} items={rowItems(menu.entry)} onClose={() => setMenu(null)} testId="aiws-directory-row-actions" />}

        {dialog?.kind === 'rename' && <NameDialog title={t('aiworkspace.directory.renameTitle', { name: dialog.entry.name })} initial={dialog.entry.name} confirm={t('aiworkspace.common.save')}
          check={(name) => nameError(members, place.prefix, name, dialog.entry.name)} onSubmit={(name) => renameEntry(store, place, dialog.entry, name)} onClose={() => setDialog(null)} />}
        {dialog?.kind === 'mkdir' && <NameDialog title={t('aiworkspace.directory.newFolderTitle')} initial={freeName(t('aiworkspace.directory.newFolderName'))}
          confirm={t('aiworkspace.directory.create')} check={(name) => nameError(members, place.prefix, name)} onSubmit={async (name) => { await createSubdirectory(store, place, name) }} onClose={() => setDialog(null)} testId="aiws-directory-mkdir" />}
        {dialog?.kind === 'existing' && <ExistingDialog exclude={new Set([rootId, ...crumbs.flatMap((c) => (c.kind === 'dir' ? [c.entityId] : []))])} check={(name) => nameError(members, place.prefix, name)}
          onAdd={(id, name) => addExistingDirectory(store, place, id, name)} onClose={() => setDialog(null)} />}
        {dialog?.kind === 'remove' && <ConfirmDialog title={dialog.entries.length === 1 ? t('aiworkspace.directory.removeTitle', { name: dialog.entries[0].name }) : t('aiworkspace.directory.removeTitleMany', { count: dialog.entries.length })}
          body={t(dialog.entries.some((e) => e.kind === 'dir') ? 'aiworkspace.directory.removeBodyDir' : 'aiworkspace.directory.removeBody')} confirm={t('aiworkspace.directory.remove')}
          onConfirm={async () => { await removeEntries(store, place, dialog.entries); setSelecting(false); setSelected(new Set()); if (preview && dialog.entries.some((e) => e.kind === 'file' && e.key === preview.key)) setPreview(null) }} onClose={() => setDialog(null)} />}
        {dialog?.kind === 'move' && <MoveDialog rootId={rootId} rootName={rootName} crumbs={crumbs} from={place} moving={dialog.entries} onPick={(to) => doMove(dialog.entries, to)} onClose={() => setDialog(null)} />}
        {dialog?.kind === 'conflict' && <ConflictDialog names={dialog.names} onDone={dialog.done} />}
        {dialog?.kind === 'linkless' && <DirDialog label={t('aiworkspace.directory.copyLink')} testId="aiws-directory-linkless" onClose={() => setDialog(null)} actions={<button type="button" className="is-primary" onClick={() => setDialog(null)}>{t('aiworkspace.common.ok')}</button>}>
          <p className="aiws-muted">{t('aiworkspace.directory.noLink')}</p>
        </DirDialog>}
      </section>
    </LockScope>
  )
}
