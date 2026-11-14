/* The home (UI framework PRD §9): workspace and public class cards in three categories — Recent, All workspaces, All
 * public classes — with search, filter settings, New (the same dialog as inside a workspace) and Import (a package
 * chosen in the system file picker, Zone first). A click on a card opens it; everything else about a workspace is in
 * its card's ⋯ menu. A phone gets one column of compact cards, a fixed category control, search behind an icon, the
 * filters in a bottom panel and New / Import as floating buttons (§9.4).
 *
 * The cards carry what `ws.list` gives (K6: owner, last modification, public, live classes) and `session.list_open`
 * lists every public class I may open (K7). "Recent" is the service's record of what I opened, on any device (with
 * what this window opened a moment ago). The category and the filters are my preference across Workspaces and devices
 * (home/prefs.ts). Covers are not generated yet: the cards draw a placeholder. */

import { useCallback, useMemo, useState } from 'react'
import { Plus, RefreshCw, Search, SlidersHorizontal, Upload, X } from 'lucide-react'
import { unwrap, type AiwsClient } from '../api/client'
import { describeError } from '../api/session'
import type { WorkspaceListItem } from '../api/types'
import { OfflineUnavailable, offlineUnavailableReason, preparedIndex, prepareOffline } from '../offline/holder'
import { SessionCard } from '../session/MySessions'
import type { OpenSession } from '../session/types'
import { useLoad } from '../state/hooks'
import { forgetOpened, noteOpened, readRecentList, type RecentEntry } from '../state/recent'
import { useAiwsText } from '../useAiwsText'
import { loadHomeSessions, openSession, openWorkspaceInNewTab } from './home/actions'
import { NewWorkspaceCard, PreparedCard, WorkspaceCard } from './home/cards'
import { FilterButton, FilterSheet } from './home/Filters'
import { DeleteDialog, DetailsDialog } from './home/HomeDialogs'
import { filterSessions, filterWorkspaces, recentItems, sortSessions, sortWorkspaces, type WorkspaceFacts } from './home/model'
import { clearedFilters, filtersActive, useHomePrefs, type HomeCategory, type HomePrefs } from './home/prefs'
import { ExportDialog, ImportDialog, NewDialog, PACKAGE_ACCEPT, type NewTab } from './shell/dialogs'
import { useFilePicker } from './shell/filePicker'
import { usePhone } from './shell/shellContext'
import './home/home.css'

/** `restricted`: a Limited account or an External DID — it lists what was shared with it and creates, imports or
 * clones nothing (the service refuses those; hybrid SSO PRE-03). */
interface Props { client: AiwsClient; onOpen: (workspaceId: string) => void; opening: string | null; restricted?: boolean }

type CardAction = 'clone' | 'delete' | 'prepare'
const ACTION_FAILED: Record<'clone' | 'delete', string> = { clone: 'aiworkspace.core.list.cloneFailed', delete: 'aiworkspace.core.list.deleteFailed' }
const CATEGORIES: HomeCategory[] = ['recent', 'all', 'sessions']
const CATEGORY_KEY: Record<HomeCategory, string> = { recent: 'aiworkspace.home.tab.recent', all: 'aiworkspace.home.tab.all', sessions: 'aiworkspace.home.tab.sessions' }

type Message = { kind: 'info' | 'error'; text: string }

export function WorkspaceList({ client, onOpen, opening, restricted = false }: Props) {
  const { t, compare } = useAiwsText()
  const phone = usePhone()
  const transport = client.transport
  /** Bumped by the refresh button: everything reloads. */
  const [version, setVersion] = useState(0)
  const load = useCallback(async () => unwrap(await client.wsList()).workspaces, [client])
  const list = useLoad<WorkspaceListItem[]>(load, version)
  const loadSessions = useCallback(() => loadHomeSessions(transport), [transport])
  const sessions = useLoad<OpenSession[]>(loadSessions, version)
  const loadRecent = useCallback(() => readRecentList(transport), [transport])
  const recent = useLoad<RecentEntry[]>(loadRecent, version)
  const loadMe = useCallback(() => transport.principal(), [transport])
  const me = useLoad<string | null>(loadMe, version)
  const home = useHomePrefs(transport)
  const prefs = home.prefs
  const setPrefs = (next: HomePrefs) => home.set(next)
  const choose = (category: HomeCategory) => home.set({ ...prefs, category })
  const [query, setQuery] = useState('')
  const [searching, setSearching] = useState(false)
  const [sheet, setSheet] = useState(false)
  const [busy, setBusy] = useState<CardAction | null>(null)
  const [busyText, setBusyText] = useState('')
  const [message, setMessage] = useState<Message | null>(null)
  const [creating, setCreating] = useState<NewTab | null>(null)
  const [importing, setImporting] = useState<File | null>(null)
  const [exporting, setExporting] = useState<WorkspaceListItem | null>(null)
  const [details, setDetails] = useState<WorkspaceListItem | null>(null)
  const [deleting, setDeleting] = useState<WorkspaceListItem | null>(null)
  /** The offline index is this browser's: read again after a card prepared a workspace. */
  const [preparedTick, setPreparedTick] = useState(0)
  const pickFile = useFilePicker((text) => setMessage({ kind: 'error', text }))

  const offline = list.error !== null && !list.data
  const listState = list.loading ? 'loading' : offline ? 'offline' : 'ready'
  const prepared = useMemo(() => { void preparedTick; return preparedIndex() }, [preparedTick])
  const facts = useMemo<WorkspaceFacts>(() => ({ me: me.data ?? null, prepared: new Set(Object.keys(prepared)) }), [me.data, prepared])
  const recentList = useMemo(() => recentItems(list.data, sessions.data, recent.data ?? []), [recent.data, list.data, sessions.data])
  // the category kept last; without one, Recent when there is any, else All workspaces (§9.1), once that is known
  const settled = !list.loading && !recent.loading && !sessions.loading && !me.loading && home.ready
  const category: HomeCategory | null = settled ? prefs.category ?? (recentList.length > 0 ? 'recent' : 'all') : null

  const openWorkspace = (workspaceId: string) => {
    void noteOpened(transport, 'workspace', workspaceId).catch(() => undefined).then(() => onOpen(workspaceId))
  }
  const openClass = (session: OpenSession) => {
    if (openSession(session)) void noteOpened(transport, 'session', session.session_id).then(() => recent.reload())
  }
  const refresh = () => setVersion((n) => n + 1)

  const act = async (action: CardAction, text: string, work: () => Promise<string | void>, failed: (error: unknown) => string) => {
    setBusy(action)
    setBusyText(text)
    setMessage(null)
    try {
      const done = await work()
      if (done) setMessage({ kind: 'info', text: done })
      list.reload()
    } catch (error) {
      setMessage({ kind: 'error', text: failed(error) })
    } finally {
      setBusy(null)
    }
  }
  const failedAs = (action: 'clone' | 'delete') => (error: unknown) => t(ACTION_FAILED[action], { error: describeError(error) })

  const cardActions = (workspace: WorkspaceListItem) => {
    const ws = { workspace_id: workspace.workspace_id }
    return {
      openTab: () => {
        openWorkspaceInNewTab(client, workspace.workspace_id, t('aiworkspace.home.noPage'))
          .then(() => noteOpened(transport, 'workspace', workspace.workspace_id))
          .catch((error: unknown) => setMessage({ kind: 'error', text: t('aiworkspace.home.openTabFailed', { error: describeError(error) }) }))
      },
      clone: () => { void act('clone', t('aiworkspace.core.list.cloning'), async () => t('aiworkspace.core.list.cloned', { id: unwrap(await client.wsClone(ws)).workspace_id }), failedAs('clone')) },
      exportPackage: () => setExporting(workspace),
      prepare: () => {
        void act('prepare', t('aiworkspace.home.preparing', { name: workspace.title }), async () => {
          const report = await prepareOffline(client, workspace.workspace_id)
          setPreparedTick((n) => n + 1)
          return t('aiworkspace.home.prepared', { name: workspace.title, count: report.assets.stored.length })
        }, (error) => (error instanceof OfflineUnavailable ? t('aiworkspace.core.app.offlineUnavailable', { reason: error.message }) : t('aiworkspace.core.app.prepareFailed', { error: describeError(error) })))
      },
      details: () => setDetails(workspace),
      remove: () => setDeleting(workspace),
    }
  }
  const confirmDelete = (workspace: WorkspaceListItem) => {
    void act('delete', t('aiworkspace.core.list.deleting'), async () => {
      unwrap(await client.wsDelete({ workspace_id: workspace.workspace_id }))
      await forgetOpened(transport, 'workspace', workspace.workspace_id)
      recent.reload()
      setDeleting(null)
      return t('aiworkspace.home.deleted', { name: workspace.title })
    }, failedAs('delete'))
  }
  const startImport = () => { void pickFile.file(PACKAGE_ACCEPT).then((file) => { if (file) setImporting(file) }) }
  const offlineReason = useMemo(() => offlineUnavailableReason(), [])

  const workspaceCard = (workspace: WorkspaceListItem, openedAt?: string) => (
    <WorkspaceCard key={`ws:${workspace.workspace_id}`} workspace={workspace} me={facts.me} openedAt={openedAt} prepared={facts.prepared.has(workspace.workspace_id)}
      opening={opening === workspace.workspace_id} busy={busy !== null} restricted={restricted} offlineReason={offlineReason}
      onOpen={() => openWorkspace(workspace.workspace_id)} actions={cardActions(workspace)} />
  )
  const sessionCard = (session: OpenSession) => <SessionCard key={`s:${session.session_id}`} session={session} onOpen={openClass} />

  // what the category shows, filtered by the search and the settings
  const filtered = !category ? [] : category === 'all'
    ? sortWorkspaces(filterWorkspaces(list.data ?? [], prefs, query, facts), prefs, compare).map((ws) => workspaceCard(ws))
    : category === 'sessions'
      ? filterSessions(sortSessions(sessions.data ?? []), prefs, query).map(sessionCard)
      : recentList.flatMap((item) => item.kind === 'workspace'
        ? filterWorkspaces([item.workspace], prefs, query, facts).map((ws) => workspaceCard(ws, item.at))
        : filterSessions([item.session], prefs, query).map(sessionCard))
  const unfiltered = !category ? 0 : category === 'all' ? (list.data?.length ?? 0) : category === 'sessions' ? (sessions.data?.length ?? 0) : recentList.length
  const narrowed = query.trim() !== '' || (category !== null && filtersActive(category, prefs))

  const tabs = (
    <div className={`aiws-home-tabs${phone ? ' is-phone' : ''}`} role="tablist" aria-label={t('aiworkspace.home.categories')} data-testid="aiws-home-tabs">
      {CATEGORIES.map((c) => (
        <button key={c} type="button" role="tab" aria-selected={category === c} data-testid={`aiws-home-tab-${c}`} onClick={() => choose(c)}>{t(CATEGORY_KEY[c])}</button>
      ))}
    </div>
  )
  const searchInput = (
    <input type="search" className="aiws-home-search" data-testid="aiws-home-search" aria-label={t('aiworkspace.home.search')} placeholder={t('aiworkspace.home.searchPlaceholder')}
      value={query} autoFocus={phone} enterKeyHint="search" onChange={(event) => setQuery(event.target.value)} />
  )
  const refreshButton = <button type="button" className="aiws-home-icon" aria-label={t('aiworkspace.common.refresh')} title={t('aiworkspace.common.refresh')} data-testid="aiws-home-refresh" onClick={refresh}><RefreshCw size={16} /></button>

  return (
    <div className={`aiws-list aiws-home${phone ? ' is-phone' : ''}`} data-testid="aiws-list" data-state={listState} data-category={category ?? undefined}>
      {phone ? (
        searching ? (
          <header className="aiws-topbar aiws-home-top is-searching">
            {searchInput}
            <button type="button" className="aiws-home-icon" aria-label={t('aiworkspace.home.searchClose')} data-testid="aiws-home-search-close" onClick={() => { setSearching(false); setQuery('') }}><X size={18} /></button>
          </header>
        ) : (
          <header className="aiws-topbar aiws-home-top">
            <b className="aiws-title">{t('aiworkspace.core.list.title')}</b>
            <span className="aiws-grow" />
            <button type="button" className="aiws-home-icon" aria-label={t('aiworkspace.home.search')} data-testid="aiws-home-search-toggle" data-active={query ? true : undefined} onClick={() => setSearching(true)}><Search size={18} /></button>
            {category && <button type="button" className="aiws-home-icon" aria-label={t('aiworkspace.home.filter')} data-testid="aiws-home-filter" data-active={filtersActive(category, prefs) || undefined} onClick={() => setSheet(true)}><SlidersHorizontal size={18} /></button>}
            {refreshButton}
          </header>
        )
      ) : (
        <header className="aiws-topbar aiws-home-top">
          <b className="aiws-title">{t('aiworkspace.core.list.title')}</b>
          <span className="aiws-muted" data-testid="aiws-transport">{transport.mode === 'dev-override' ? t('aiworkspace.core.list.transportDev', { principal: transport.principalHint ?? '' }) : t('aiworkspace.core.list.transportZone')}</span>
          <span className="aiws-grow" />
          {searchInput}
          {category && <FilterButton category={category} prefs={prefs} onChange={setPrefs} />}
          {refreshButton}
          {!restricted && <button type="button" data-testid="aiws-home-import" onClick={startImport}><Upload size={16} /> {t('aiworkspace.home.import')}</button>}
          {!restricted && <button type="button" className="is-primary" data-testid="aiws-new" onClick={() => setCreating('workspace')}><Plus size={16} /> {t('aiworkspace.core.list.new')}</button>}
        </header>
      )}
      {!offline && tabs}
      <div className="aiws-home-scroll">
        {message && <div className={message.kind === 'error' ? 'aiws-error' : 'aiws-warning'} role={message.kind === 'error' ? 'alert' : 'status'} data-testid="aiws-list-message">{message.text}</div>}
        {busy && <div className="aiws-muted" role="status" data-testid="aiws-list-busy">{busyText}</div>}
        {list.error && <div className="aiws-error" role="alert" data-testid="aiws-list-error">{t('aiworkspace.core.list.loadFailed', { error: list.error })}</div>}
        {offline ? <PreparedList onOpen={onOpen} opening={opening} principal={transport.principalHint} />
          : !category ? <div className="aiws-muted aiws-home-empty" role="status" data-testid="aiws-home-loading">{t('aiworkspace.home.loading')}</div>
            : (
              <>
                {category === 'sessions' && sessions.error && <div className="aiws-error" role="alert" data-testid="aiws-home-sessions-error">{t('aiworkspace.session.mine.loadFailed', { error: sessions.error })}</div>}
                <div className="aiws-home-grid" data-testid={`aiws-home-grid-${category}`}>
                  {category === 'all' && !restricted && !narrowed && <NewWorkspaceCard onNew={() => setCreating('workspace')} onTemplate={() => setCreating('template')} />}
                  {filtered}
                </div>
                {filtered.length === 0 && !(category === 'sessions' && sessions.error) && (narrowed && unfiltered > 0
                  ? <div className="aiws-muted aiws-home-empty" data-testid="aiws-home-no-match">{t('aiworkspace.home.noMatch')} <button type="button" className="aiws-link" data-testid="aiws-home-clear" onClick={() => { setQuery(''); setPrefs(clearedFilters(prefs)) }}>{t('aiworkspace.home.clearFilters')}</button></div>
                  : <div className="aiws-muted aiws-home-empty" data-testid="aiws-home-empty">{t(category === 'all' ? 'aiworkspace.core.list.empty' : category === 'sessions' ? 'aiworkspace.home.sessions.empty' : 'aiworkspace.home.recent.empty')}</div>)}
              </>
            )}
      </div>
      {phone && !restricted && (
        <div className="aiws-home-fabs">
          <button type="button" className="aiws-home-fab" data-testid="aiws-home-import" aria-label={t('aiworkspace.home.import')} title={t('aiworkspace.home.import')} onClick={startImport}><Upload size={20} /></button>
          <button type="button" className="aiws-home-fab is-primary" data-testid="aiws-new" aria-label={t('aiworkspace.core.list.new')} title={t('aiworkspace.core.list.new')} onClick={() => setCreating('workspace')}><Plus size={24} /></button>
        </div>
      )}
      {sheet && category && <FilterSheet category={category} prefs={prefs} onChange={setPrefs} onClose={() => setSheet(false)} />}
      {creating && <NewDialog client={client} store={null} initialTab={creating} onClose={() => setCreating(null)} onOpenWorkspace={openWorkspace} />}
      {importing && <ImportDialog client={client} file={importing} stayLabel={t('aiworkspace.common.close')} onClose={() => setImporting(null)} onOpenWorkspace={openWorkspace}
        onDone={(text) => { setMessage({ kind: 'info', text }); list.reload() }} />}
      {exporting && <ExportDialog client={client} workspace={exporting} onClose={() => setExporting(null)} />}
      {details && <DetailsDialog workspace={details} me={facts.me} prepared={prepared[details.workspace_id]} onClose={() => setDetails(null)} />}
      {deleting && <DeleteDialog workspace={deleting} busy={busy !== null} onConfirm={() => confirmDelete(deleting)} onClose={() => setDeleting(null)} />}
    </div>
  )
}

/** The backend cannot be reached: the Workspaces prepared for offline on this device can still be opened, as cards. */
function PreparedList({ onOpen, opening, principal }: { onOpen: (workspaceId: string) => void; opening: string | null; principal: string | null }) {
  const { t } = useAiwsText()
  const prepared = Object.values(preparedIndex()).filter((hint) => !principal || hint.principal === principal)
  return (
    <div className="aiws-home-prepared" data-testid="aiws-prepared-list">
      <div className="aiws-warning" role="status">
        {t('aiworkspace.core.list.preparedStatus', {
          network: navigator.onLine === false ? t('aiworkspace.core.list.browserOffline') : t('aiworkspace.core.list.backendUnreachable'),
          prepared: prepared.length > 0 ? t('aiworkspace.core.list.preparedHint') : t('aiworkspace.core.list.noneOffline'),
        })}
      </div>
      <div className="aiws-home-grid">
        {prepared.map((hint) => <PreparedCard key={hint.workspace_id} hint={hint} opening={opening === hint.workspace_id} onOpen={() => onOpen(hint.workspace_id)} />)}
      </div>
    </div>
  )
}
