import { useMemo, useState, useSyncExternalStore } from 'react'
import { useMediaQuery } from '@mui/material'
import useSWR from 'swr'
import {
  AlertTriangle,
  ArrowLeft,
  ChevronDown,
  ChevronRight,
  Crown,
  Loader2,
  RefreshCw,
  RotateCcw,
  Search,
  SlidersHorizontal,
  Trash2,
  Trophy,
} from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { useAICCStore } from './hooks/use-aicc-store'
import { LongField } from './components/shared/LongField'
import { CommandLabel, KindIcon } from './components/routing/RoutingKind'
import { RouteTraceAuditPanel } from './components/usage/RouteTraceAuditPanel'
import {
  candidateTree,
  commandStatus,
  commandSubject,
  commandValue,
  findCommand,
  isExactTarget,
  KIND_STYLE,
  kindLabel,
  modelOfExact,
  namespaceOf,
  providerOfExact,
  targetBase,
  upsertCommand,
  winReason,
  type CandidateTreeNode,
  type DirectoryKind,
  type ExpansionState,
  type RoutePreview,
  type RoutingCommand,
  type RoutingDirectory,
  type RoutingWorkspace,
} from './datamodel/routing'

type Translate = ReturnType<typeof useI18n>['t']
type ListFilter = 'all' | 'available' | 'spec' | 'task'
type DetailMode = 'candidates' | 'advanced'

const surface = { background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }
const muted = { color: 'var(--cp-muted)' }

function sourceLabel(source: string, t: Translate): string {
  return t(`aiCenter.routing.source.${source}`, source)
}

function stateLabel(state: ExpansionState, t: Translate): string {
  return t(`aiCenter.routing.state.${state}`, state)
}

export function RoutingPage() {
  const { t } = useI18n()
  const store = useAICCStore()
  const version = useSyncExternalStore(store.subscribe, store.getSnapshotVersion)
  const isMobile = useMediaQuery('(max-width: 767px)')
  const { data, error, isLoading, isValidating, mutate } = useSWR(
    ['aicc-routing-workspace', store, version],
    () => store.fetchRoutingWorkspace(),
    { refreshInterval: 30000, keepPreviousData: true },
  )
  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<ListFilter>('available')
  const [selectedPath, setSelectedPath] = useState<string | null>(null)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  const directories = useMemo(() => data?.directories ?? [], [data])
  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase()
    return directories
      .filter((directory) => {
        if (filter === 'available' && !directory.available) return false
        if (filter === 'spec' && directory.kind !== 'spec') return false
        if (filter === 'task' && directory.kind !== 'task') return false
        if (!needle) return true
        return [directory.path, directory.selectedExactModel ?? '', ...directory.items.map((item) => item.target)]
          .some((value) => value.toLowerCase().includes(needle))
      })
      .sort((left, right) => Number(right.available) - Number(left.available) || left.path.localeCompare(right.path))
  }, [directories, filter, query])
  const selected = directories.find((directory) => directory.path === selectedPath)
    ?? (isMobile ? undefined : visible[0])

  const saveCommands = async (commands: RoutingCommand[]) => {
    if (!data) return
    setSaving(true)
    setSaveError(null)
    try {
      await store.saveRoutingCommands(commands, data.settingsRevision)
      await mutate()
    } catch (cause) {
      setSaveError(cause instanceof Error ? cause.message : String(cause))
      await mutate()
    } finally {
      setSaving(false)
    }
  }

  const header = (
    <header className="flex items-start justify-between gap-3">
      <div className="min-w-0">
        <h2 className="text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.routing.title', 'Routing')}</h2>
        <p className="mt-1 max-w-3xl text-sm leading-6" style={muted}>
          {t('aiCenter.routing.subtitle', 'See which model each directory routes to right now and why. Adjustments are saved as commands that are re-applied whenever the logical tree is rebuilt.')}
        </p>
      </div>
      <button
        type="button"
        onClick={() => void mutate()}
        disabled={isValidating}
        aria-label={t('aiCenter.routing.refresh', 'Refresh routing')}
        title={t('aiCenter.routing.refresh', 'Refresh routing')}
        className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg disabled:opacity-50"
        style={surface}
      >
        <RefreshCw size={17} className={isValidating ? 'animate-spin' : ''} />
      </button>
    </header>
  )

  if (error && !data) {
    return (
      <div className="flex flex-col gap-4">
        {header}
        <div role="alert" className="flex flex-wrap items-center justify-between gap-2 rounded-lg p-3 text-sm" style={{ ...surface, color: 'var(--cp-danger)' }}>
          {t('aiCenter.routing.loadFailed', 'Could not load the routing directory.')}
          <button type="button" className="min-h-11 px-3 underline" onClick={() => void mutate()}>{t('common.retry', 'Retry')}</button>
        </div>
      </div>
    )
  }
  if (isLoading && !data) {
    return (
      <div className="flex flex-col gap-4">
        {header}
        <div role="status" className="flex items-center justify-center gap-2 py-16 text-sm" style={muted}>
          <Loader2 size={20} className="animate-spin" />{t('aiCenter.routing.loading', 'Loading routing directory…')}
        </div>
      </div>
    )
  }

  const detail = selected && data ? (
    <DirectoryDetail
      key={selected.path}
      directory={selected}
      workspace={data}
      directories={directories}
      saving={saving}
      onSave={saveCommands}
      onOpen={(path) => setSelectedPath(path)}
    />
  ) : null

  return (
    <div className="flex min-w-0 flex-col gap-4" style={{ color: 'var(--cp-text)' }}>
      {header}
      {saveError && (
        <div role="alert" className="flex items-start gap-2 rounded-lg p-3 text-sm" style={{ ...surface, color: 'var(--cp-danger)' }}>
          <AlertTriangle size={16} className="mt-0.5 shrink-0" />
          <span className="min-w-0 break-words">{t('aiCenter.routing.saveFailed', 'Saving the adjustment failed: {{error}}', { error: saveError })}</span>
        </div>
      )}
      {data && <AdjustmentsPanel workspace={data} saving={saving} onSave={saveCommands} onOpen={setSelectedPath} />}
      {isMobile && selected ? (
        <div className="flex flex-col gap-3">
          <button type="button" onClick={() => setSelectedPath(null)} className="inline-flex min-h-11 items-center gap-1 self-start text-sm" style={{ color: 'var(--cp-accent)' }}>
            <ArrowLeft size={16} />{t('aiCenter.routing.backToList', 'All directories')}
          </button>
          {detail}
        </div>
      ) : (
        <div className={isMobile ? 'flex flex-col gap-4' : 'grid grid-cols-[minmax(260px,340px)_minmax(0,1fr)] items-start gap-4'}>
          <DirectoryList
            directories={visible}
            total={directories.length}
            query={query}
            filter={filter}
            selectedPath={selected?.path}
            onQuery={setQuery}
            onFilter={setFilter}
            onSelect={setSelectedPath}
          />
          {!isMobile && (detail ?? (
            <div className="rounded-xl p-8 text-center text-sm" style={{ ...surface, ...muted }}>
              {t('aiCenter.routing.selectDirectory', 'Select a directory to inspect its routing.')}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

function DirectoryList({
  directories,
  total,
  query,
  filter,
  selectedPath,
  onQuery,
  onFilter,
  onSelect,
}: {
  directories: RoutingDirectory[]
  total: number
  query: string
  filter: ListFilter
  selectedPath?: string
  onQuery: (value: string) => void
  onFilter: (value: ListFilter) => void
  onSelect: (path: string) => void
}) {
  const { t } = useI18n()
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set())
  const groups = useMemo(() => {
    const map = new Map<string, RoutingDirectory[]>()
    for (const directory of directories) {
      const namespace = namespaceOf(directory.path)
      map.set(namespace, [...(map.get(namespace) ?? []), directory])
    }
    return [...map.entries()].sort(([left], [right]) => (left === 'llm' ? -1 : right === 'llm' ? 1 : left.localeCompare(right)))
  }, [directories])
  const filters: [ListFilter, string][] = [
    ['all', t('aiCenter.routing.filter.all', 'All')],
    ['available', t('aiCenter.routing.filter.available', 'Available')],
    ['spec', t('aiCenter.routing.filter.spec', 'Specifications')],
    ['task', t('aiCenter.routing.filter.task', 'Use cases')],
  ]

  return (
    <section className="flex min-w-0 flex-col overflow-hidden rounded-xl md:sticky md:top-4 md:max-h-[calc(100dvh-8rem)]" style={surface} aria-label={t('aiCenter.routing.directories', 'Directories')}>
      <div className="flex flex-col gap-2 p-3" style={{ borderBottom: '1px solid var(--cp-border)' }}>
        <label className="flex min-h-10 items-center gap-2 rounded-lg px-3" style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)' }}>
          <Search size={15} style={muted} />
          <input
            value={query}
            onChange={(event) => onQuery(event.target.value)}
            placeholder={t('aiCenter.routing.search', 'Search directory or model')}
            aria-label={t('aiCenter.routing.search', 'Search directory or model')}
            className="min-w-0 flex-1 bg-transparent py-2 text-sm outline-none"
            style={{ color: 'var(--cp-text)' }}
          />
        </label>
        <div role="radiogroup" aria-label={t('aiCenter.routing.filterLabel', 'Directory filter')} className="grid grid-cols-4 gap-1 rounded-lg p-1" style={{ background: 'var(--cp-bg)' }}>
          {filters.map(([value, label]) => (
            <button
              key={value}
              type="button"
              role="radio"
              aria-checked={filter === value}
              onClick={() => onFilter(value)}
              className="min-h-8 truncate rounded-md px-1 text-xs"
              style={{
                background: filter === value ? 'var(--cp-surface)' : 'transparent',
                color: filter === value ? 'var(--cp-text)' : 'var(--cp-muted)',
                border: filter === value ? '1px solid var(--cp-border)' : '1px solid transparent',
              }}
            >
              {label}
            </button>
          ))}
        </div>
        <div className="flex flex-wrap items-center justify-between gap-2 text-[11px]" style={muted}>
          <span>{t('aiCenter.routing.listCount', '{{count}} of {{total}} directories', { count: directories.length, total })}</span>
          <span className="flex items-center gap-2">
            {(['task', 'family', 'spec'] as DirectoryKind[]).map((kind) => (
              <span key={kind} className="inline-flex items-center gap-1">
                <span className="h-2 w-2 rounded-full" style={{ background: KIND_STYLE[kind].color }} />{kindLabel(kind, t)}
              </span>
            ))}
          </span>
        </div>
      </div>
      <div className="min-h-0 overflow-y-auto p-2">
        {groups.length === 0 && (
          <p className="px-2 py-8 text-center text-sm" style={muted}>{t('aiCenter.routing.noMatches', 'No directory matches the current filter.')}</p>
        )}
        {groups.map(([namespace, entries]) => {
          const isCollapsed = collapsed.has(namespace)
          return (
            <div key={namespace} className="mb-1">
              <button
                type="button"
                aria-expanded={!isCollapsed}
                onClick={() => setCollapsed((current) => {
                  const next = new Set(current)
                  if (next.has(namespace)) next.delete(namespace)
                  else next.add(namespace)
                  return next
                })}
                className="flex min-h-8 w-full items-center gap-1 rounded-md px-2 text-left text-xs font-semibold uppercase tracking-wide"
                style={muted}
              >
                {isCollapsed ? <ChevronRight size={13} /> : <ChevronDown size={13} />}
                <span className="font-mono">{namespace}</span>
                <span className="ml-auto font-normal tabular-nums">{entries.length}</span>
              </button>
              {!isCollapsed && entries.map((directory) => {
                const active = directory.path === selectedPath
                return (
                  <button
                    key={directory.path}
                    type="button"
                    data-testid="routing-directory"
                    data-path={directory.path}
                    data-kind={directory.kind}
                    data-available={directory.available}
                    onClick={() => onSelect(directory.path)}
                    aria-current={active}
                    className="flex min-h-11 w-full min-w-0 items-center gap-2 rounded-lg px-2 py-1.5 text-left"
                    style={{
                      background: active ? 'var(--cp-surface-2)' : 'transparent',
                      border: active ? '1px solid var(--cp-border)' : '1px solid transparent',
                      opacity: directory.available ? 1 : 0.6,
                    }}
                  >
                    <KindIcon kind={directory.kind} size={13} />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-mono text-xs font-medium" style={{ color: 'var(--cp-text)' }}>{directory.path}</span>
                      <span className="block truncate text-[11px]" style={muted}>
                        {directory.selectedExactModel ?? t('aiCenter.routing.unavailable', 'Unavailable')}
                      </span>
                    </span>
                    <span
                      className="h-2 w-2 shrink-0 rounded-full"
                      title={directory.available ? t('aiCenter.routing.available', 'Available') : t('aiCenter.routing.unavailable', 'Unavailable')}
                      style={{ background: directory.available ? 'var(--cp-success)' : 'var(--cp-muted)' }}
                    />
                  </button>
                )
              })}
            </div>
          )
        })}
      </div>
    </section>
  )
}

function DirectoryDetail({
  directory,
  workspace,
  directories,
  saving,
  onSave,
  onOpen,
}: {
  directory: RoutingDirectory
  workspace: RoutingWorkspace
  directories: RoutingDirectory[]
  saving: boolean
  onSave: (commands: RoutingCommand[]) => Promise<void>
  onOpen: (path: string) => void
}) {
  const { t } = useI18n()
  const store = useAICCStore()
  const [tab, setTab] = useState<'route' | 'traces'>('route')
  const [mode, setMode] = useState<DetailMode>('candidates')
  const [showSkipped, setShowSkipped] = useState(false)
  const { data: preview, error, isLoading } = useSWR(
    ['aicc-routing-preview', store, directory.path, workspace.settingsRevision, workspace.directories],
    () => store.previewRoute(directory.path),
    { keepPreviousData: false },
  )
  const kinds = useMemo(() => new Map(directories.map((entry) => [entry.path, entry.kind])), [directories])

  return (
    <section className="flex min-w-0 flex-col gap-4" aria-label={directory.path}>
      <WinnerHeader directory={directory} preview={preview} loading={isLoading} error={error} />
      <div className="flex min-h-10 items-center gap-1 rounded-xl p-1" style={surface} role="tablist">
        {([
          ['route', t('aiCenter.routing.tab.route', 'Routing details')],
          ['traces', t('aiCenter.routing.tab.traces', 'Recent traces')],
        ] as const).map(([value, label]) => (
          <button
            key={value}
            type="button"
            role="tab"
            aria-selected={tab === value}
            onClick={() => setTab(value)}
            className="min-h-8 flex-1 rounded-lg px-3 text-xs font-medium"
            style={{
              background: tab === value ? 'var(--cp-surface-2)' : 'transparent',
              color: tab === value ? 'var(--cp-text)' : 'var(--cp-muted)',
              border: tab === value ? '1px solid var(--cp-border)' : '1px solid transparent',
            }}
          >
            {label}
          </button>
        ))}
      </div>
      {tab === 'traces' ? (
        <RouteTraceAuditPanel
          compact={false}
          logicalPathFilter={directory.path}
          activeTraceId={null}
          onTraceSelect={() => undefined}
          onClearLogicalPathFilter={() => undefined}
        />
      ) : (
        <section className="flex flex-col gap-3 rounded-xl p-4" style={surface}>
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h3 className="text-sm font-semibold">
              {mode === 'candidates'
                ? t('aiCenter.routing.candidatesTitle', 'Candidates by path')
                : t('aiCenter.routing.itemsTitle', 'Items in this directory')}
            </h3>
            <div className="flex items-center gap-2">
              {mode === 'candidates' && (
                <label className="flex min-h-8 cursor-pointer items-center gap-1.5 text-xs" style={muted}>
                  <input type="checkbox" checked={showSkipped} onChange={(event) => setShowSkipped(event.target.checked)} style={{ accentColor: 'var(--cp-accent)' }} />
                  {t('aiCenter.routing.showSkipped', 'Show skipped items')}
                </label>
              )}
              <button
                type="button"
                aria-pressed={mode === 'advanced'}
                onClick={() => setMode(mode === 'advanced' ? 'candidates' : 'advanced')}
                className="inline-flex min-h-8 items-center gap-1.5 rounded-lg px-2.5 text-xs"
                style={{ ...surface, color: mode === 'advanced' ? 'var(--cp-accent)' : 'var(--cp-muted)', borderColor: mode === 'advanced' ? 'var(--cp-accent)' : 'var(--cp-border)' }}
              >
                <SlidersHorizontal size={13} />
                {t('aiCenter.routing.advancedMode', 'Advanced mode')}
              </button>
            </div>
          </div>
          {mode === 'advanced' ? (
            <ItemsEditor directory={directory} workspace={workspace} preview={preview} kinds={kinds} saving={saving} onSave={onSave} onOpen={onOpen} />
          ) : preview ? (
            <CandidateTreeView preview={preview} kinds={kinds} showSkipped={showSkipped} onOpen={onOpen} />
          ) : (
            <div className="flex items-center gap-2 py-6 text-sm" style={muted}>
              <Loader2 size={16} className="animate-spin" />{t('aiCenter.routing.previewLoading', 'Resolving route…')}
            </div>
          )}
        </section>
      )}
    </section>
  )
}

function WinnerHeader({
  directory,
  preview,
  loading,
  error,
}: {
  directory: RoutingDirectory
  preview?: RoutePreview
  loading: boolean
  error: unknown
}) {
  const { t } = useI18n()
  const winner = preview?.selectedExactModel ?? directory.selectedExactModel
  const tree = preview ? candidateTree(preview) : undefined
  const winningPath: string[] = []
  let level = tree?.nodes
  while (level) {
    const next: CandidateTreeNode | undefined = level.find((node) => node.winning)
    if (!next) break
    winningPath.push(isExactTarget(next.item.target) ? next.item.target : next.item.name)
    level = next.children
  }
  const profile = preview?.schedulerProfile ?? directory.profile

  return (
    <section className="rounded-xl p-4" style={surface} data-testid="routing-winner">
      <div className="flex flex-wrap items-center gap-2">
        <KindIcon kind={directory.kind} />
        <span className="rounded-md px-2 py-0.5 text-[11px]" style={{ color: KIND_STYLE[directory.kind].color, background: `color-mix(in srgb, ${KIND_STYLE[directory.kind].color} 12%, transparent)` }}>
          {kindLabel(directory.kind, t)}
        </span>
        <LongField value={directory.path} className="text-sm font-semibold" mono />
        <span className="ml-auto text-xs" style={muted}>
          {[directory.apiType, profile && t(`aiCenter.routing.profile.${profile}`, profile)].filter(Boolean).join(' · ')}
        </span>
      </div>
      <div className="mt-3 rounded-lg p-3" style={{ background: 'var(--cp-bg)' }}>
        {winner ? (
          <>
            <div className="flex items-center gap-2 text-xs" style={muted}>
              <Trophy size={14} style={{ color: 'var(--cp-warning)' }} />{t('aiCenter.routing.winner', 'Current winner')}
            </div>
            <div className="mt-1 flex flex-wrap items-baseline gap-x-2">
              <span className="break-all font-mono text-base font-semibold" data-testid="routing-winner-model">{modelOfExact(winner)}</span>
              <span className="text-xs" style={muted}>@{providerOfExact(winner)}</span>
            </div>
            {winningPath.length > 0 && (
              <p className="mt-1 break-all font-mono text-[11px]" style={muted}>{[directory.path, ...winningPath].join(' → ')}</p>
            )}
            <p className="mt-2 text-sm leading-6" data-testid="routing-winner-reason">
              {preview ? winReasonText(preview, t) : t('aiCenter.routing.previewLoading', 'Resolving route…')}
            </p>
          </>
        ) : (
          <div className="flex items-start gap-2 text-sm">
            {loading ? <Loader2 size={16} className="mt-0.5 animate-spin" style={muted} /> : <AlertTriangle size={16} className="mt-0.5 shrink-0" style={{ color: 'var(--cp-warning)' }} />}
            <div className="min-w-0">
              <p className="font-medium">{loading ? t('aiCenter.routing.previewLoading', 'Resolving route…') : t('aiCenter.routing.noWinner', 'No model can serve this directory right now')}</p>
              {!loading && (preview?.error ?? directory.error ?? (error ? String(error) : undefined)) && (
                <p className="mt-1 break-words font-mono text-xs" style={muted}>{preview?.error ?? directory.error ?? String(error)}</p>
              )}
            </div>
          </div>
        )}
      </div>
    </section>
  )
}

function winReasonText(preview: RoutePreview, t: Translate): string {
  const reason = winReason(preview)
  const count = preview.ranked.length
  const profile = t(`aiCenter.routing.profile.${preview.schedulerProfile ?? 'balanced'}`, preview.schedulerProfile ?? 'balanced')
  const fallback = preview.fallbackChain.length
    ? `${t('aiCenter.routing.reason.fallback', 'Reached through fallback {{chain}}.', { chain: preview.fallbackChain.map((step) => `${step.from} → ${step.to}`).join(', ') })} `
    : ''
  switch (reason.code) {
    case 'only_candidate':
      return fallback + t('aiCenter.routing.reason.only', 'It is the only available candidate left after each level expanded its highest-weight items.')
    case 'best_score':
      return fallback + t('aiCenter.routing.reason.best', 'Best {{factor}} score among {{count}} candidates under the {{profile}} policy; runner-up is {{runnerUp}}.', {
        factor: t(`aiCenter.routing.score.${reason.factor}`, reason.factor),
        count,
        profile,
        runnerUp: reason.runnerUp,
      })
    case 'default_order':
      return fallback + t('aiCenter.routing.reason.order', 'Ties with {{runnerUp}} on every policy score under {{profile}}; it wins by default order.', {
        runnerUp: reason.runnerUp,
        profile,
      })
    default:
      return fallback
  }
}

function CandidateTreeView({
  preview,
  kinds,
  showSkipped,
  onOpen,
}: {
  preview: RoutePreview
  kinds: Map<string, DirectoryKind>
  showSkipped: boolean
  onOpen: (path: string) => void
}) {
  const { t } = useI18n()
  const { root, nodes } = candidateTree(preview)
  if (!root) {
    return <p className="py-4 text-sm" style={muted}>{t('aiCenter.routing.noExpansion', 'No expansion record is available for this directory.')}</p>
  }
  return (
    <div className="flex flex-col gap-1" data-testid="routing-candidate-tree">
      <TreeLevel nodes={nodes} kinds={kinds} depth={0} showSkipped={showSkipped} onOpen={onOpen} />
      {preview.ranked.length > 0 && (
        <p className="mt-2 text-[11px]" style={muted}>
          {t('aiCenter.routing.stageTwoHint', 'Candidates reached above are compared together by the {{profile}} policy; lower score wins, ties keep default order.', {
            profile: t(`aiCenter.routing.profile.${preview.schedulerProfile ?? 'balanced'}`, preview.schedulerProfile ?? 'balanced'),
          })}
        </p>
      )}
    </div>
  )
}

function TreeLevel({
  nodes,
  kinds,
  depth,
  showSkipped,
  onOpen,
}: {
  nodes: CandidateTreeNode[]
  kinds: Map<string, DirectoryKind>
  depth: number
  showSkipped: boolean
  onOpen: (path: string) => void
}) {
  const { t } = useI18n()
  const shown = showSkipped ? nodes : nodes.filter((node) => node.item.state === 'expanded')
  const hidden = nodes.length - shown.length
  return (
    <div className="flex flex-col gap-1" style={{ paddingLeft: depth ? 14 : 0, borderLeft: depth ? '1px dashed var(--cp-border)' : undefined, marginLeft: depth ? 12 : 0 }}>
      {shown.map((node) => (
        <div key={node.key} className="flex flex-col gap-1">
          <TreeRow node={node} kind={kinds.get(targetBase(node.item.target))} onOpen={onOpen} />
          {node.children.length > 0 && (
            <TreeLevel nodes={node.children} kinds={kinds} depth={depth + 1} showSkipped={showSkipped} onOpen={onOpen} />
          )}
        </div>
      ))}
      {hidden > 0 && (
        <p className="px-2 text-[11px]" style={muted}>{t('aiCenter.routing.skippedCount', '+{{count}} lower-weight or unavailable items not expanded', { count: hidden })}</p>
      )}
    </div>
  )
}

function TreeRow({ node, kind, onOpen }: { node: CandidateTreeNode; kind?: DirectoryKind; onOpen: (path: string) => void }) {
  const { t } = useI18n()
  const { item, ranked, filtered } = node
  const leaf = isExactTarget(item.target)
  const stateColor = item.state === 'expanded' ? 'var(--cp-success)' : item.state === 'unavailable' ? 'var(--cp-danger)' : 'var(--cp-muted)'
  return (
    <div
      className="flex min-w-0 flex-wrap items-center gap-2 rounded-lg px-2 py-1.5 text-xs"
      data-testid="routing-tree-row"
      data-target={item.target}
      data-state={item.state}
      style={{
        background: node.winning ? 'color-mix(in srgb, var(--cp-warning) 10%, var(--cp-bg))' : 'var(--cp-bg)',
        border: `1px solid ${node.winning ? 'color-mix(in srgb, var(--cp-warning) 45%, transparent)' : 'transparent'}`,
        opacity: item.state === 'expanded' ? 1 : 0.65,
      }}
    >
      {leaf ? <Crown size={13} style={{ color: node.winning ? 'var(--cp-warning)' : 'var(--cp-muted)', flexShrink: 0 }} /> : <KindIcon kind={kind ?? 'directory'} size={11} />}
      {leaf ? (
        <span className="min-w-0 break-all font-mono">{item.target}</span>
      ) : (
        <button type="button" onClick={() => onOpen(targetBase(item.target))} className="min-w-0 break-all text-left font-mono underline-offset-2 hover:underline" style={{ color: 'var(--cp-accent)' }}>
          {item.name === targetBase(item.target) ? item.target : `${item.name} → ${item.target}`}
        </button>
      )}
      <span className="ml-auto inline-flex shrink-0 items-center gap-2">
        <span className="tabular-nums" title={sourceLabel(item.weightSource, t)}>
          {t('aiCenter.routing.weightShort', 'w {{weight}}', { weight: formatWeight(item.weight) })}
          {item.weightSource === 'routing_command' && <span style={{ color: 'var(--cp-accent)' }}> *</span>}
        </span>
        <span className="inline-flex items-center gap-1" style={{ color: stateColor }}>
          <span className="h-1.5 w-1.5 rounded-full" style={{ background: stateColor }} />{stateLabel(item.state, t)}
        </span>
      </span>
      {leaf && ranked && (
        <span className="w-full pl-5 text-[11px]" style={muted}>
          {t('aiCenter.routing.candidateScore', 'score {{score}} · default order #{{order}}', { score: ranked.finalScore.toFixed(3), order: ranked.defaultOrder + 1 })}
          {ranked.selected && <strong style={{ color: 'var(--cp-warning)' }}> · {t('aiCenter.routing.selected', 'selected')}</strong>}
        </span>
      )}
      {leaf && filtered && (
        <span className="w-full break-words pl-5 text-[11px]" style={{ color: 'var(--cp-danger)' }}>
          {filtered.reasons.map((reason) => reason.summary).join('; ')}
        </span>
      )}
    </div>
  )
}

function ItemsEditor({
  directory,
  workspace,
  preview,
  kinds,
  saving,
  onSave,
  onOpen,
}: {
  directory: RoutingDirectory
  workspace: RoutingWorkspace
  preview?: RoutePreview
  kinds: Map<string, DirectoryKind>
  saving: boolean
  onSave: (commands: RoutingCommand[]) => Promise<void>
  onOpen: (path: string) => void
}) {
  const { t } = useI18n()
  const [drafts, setDrafts] = useState<Record<string, string>>({})
  const step = preview?.expansion.find((entry) => entry.path === directory.path)
  const states = new Map(step?.items.map((item) => [item.name, item.state]) ?? [])
  const maxWeight = Math.max(0, ...directory.items.map((item) => item.weight))
  const commandFor = (item: string): RoutingCommand => ({ kind: 'item_weight', path: directory.path, item, weight: 0 })
  const stale = workspace.status.filter((status) => status.staleReason && status.command.kind === 'item_weight' && status.command.path === directory.path)

  const apply = (item: string, weight: number) => {
    setDrafts((current) => ({ ...current, [item]: '' }))
    void onSave(upsertCommand(workspace.commands, { kind: 'item_weight', path: directory.path, item, weight }))
  }

  if (directory.items.length === 0) {
    return <p className="py-4 text-sm" style={muted}>{t('aiCenter.routing.noItems', 'This directory has no child items.')}</p>
  }
  return (
    <div className="flex flex-col gap-2" data-testid="routing-items-editor">
      <p className="text-xs leading-5" style={muted}>
        {t('aiCenter.routing.manualHint', 'Each level only expands its highest-weight available items. Set a child weight here to change which one wins at this level; the value is saved as a command and re-applied after every tree update.')}
      </p>
      {stale.map((status) => (
        <p key={commandSubject(status.command)} className="flex items-start gap-1.5 text-xs" style={{ color: 'var(--cp-warning)' }}>
          <AlertTriangle size={13} className="mt-0.5 shrink-0" />{status.staleReason}
        </p>
      ))}
      <div className="flex flex-col divide-y" style={{ borderColor: 'var(--cp-border)' }}>
        {directory.items.map((item) => {
          const command = findCommand(workspace.commands, commandFor(item.name))
          const state = states.get(item.name)
          const draft = drafts[item.name] ?? ''
          const parsed = Number(draft)
          const valid = draft.trim() !== '' && Number.isFinite(parsed) && parsed >= 0
          return (
            <div key={item.name} className="flex flex-wrap items-center gap-2 py-2" data-testid="routing-item" data-item={item.name} style={{ borderColor: 'var(--cp-border)' }}>
              <span className="flex min-w-0 flex-1 basis-56 items-center gap-2">
                {isExactTarget(item.target) ? <Crown size={13} style={muted} /> : <KindIcon kind={kinds.get(targetBase(item.target)) ?? 'directory'} size={11} />}
                <span className="min-w-0">
                  {isExactTarget(item.target) ? (
                    <span className="block break-all font-mono text-xs">{item.target}</span>
                  ) : (
                    <button type="button" onClick={() => onOpen(targetBase(item.target))} className="block break-all text-left font-mono text-xs hover:underline" style={{ color: 'var(--cp-accent)' }}>
                      {item.name === targetBase(item.target) ? item.target : `${item.name} → ${item.target}`}
                    </button>
                  )}
                  <span className="block text-[11px]" style={muted}>
                    {t('aiCenter.routing.itemMeta', 'default {{weight}} · {{source}}', { weight: formatWeight(item.defaultWeight), source: sourceLabel(item.weightSource, t) })}
                    {state && ` · ${stateLabel(state, t)}`}
                  </span>
                </span>
              </span>
              <span className="w-14 text-right font-mono text-sm tabular-nums" style={{ color: item.weight === maxWeight && item.weight > 0 ? 'var(--cp-success)' : 'var(--cp-text)' }}>
                {formatWeight(item.weight)}
              </span>
              <form
                className="flex items-center gap-1"
                onSubmit={(event) => {
                  event.preventDefault()
                  if (valid) apply(item.name, parsed)
                }}
              >
                <input
                  type="number"
                  min={0}
                  step="any"
                  value={draft}
                  placeholder={formatWeight(command ? commandValue(command) : item.weight)}
                  onChange={(event) => setDrafts((current) => ({ ...current, [item.name]: event.target.value }))}
                  aria-label={t('aiCenter.routing.weightInput', 'Weight for {{item}}', { item: item.name })}
                  className="h-8 w-20 rounded-md px-2 text-xs outline-none"
                  style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}
                />
                <button type="submit" disabled={!valid || saving} className="h-8 rounded-md px-2 text-xs disabled:opacity-40" style={{ ...surface, color: 'var(--cp-accent)' }}>
                  {t('aiCenter.routing.setWeight', 'Set')}
                </button>
                <button
                  type="button"
                  disabled={saving || (item.weight === maxWeight && directory.items.filter((entry) => entry.weight === maxWeight).length === 1)}
                  onClick={() => apply(item.name, Math.floor(maxWeight) + 1)}
                  className="h-8 rounded-md px-2 text-xs disabled:opacity-40"
                  style={{ ...surface, color: 'var(--cp-warning)' }}
                  title={t('aiCenter.routing.makeWinnerHint', 'Set a weight above every sibling so this item wins at this level')}
                >
                  {t('aiCenter.routing.makeWinner', 'Prefer')}
                </button>
                {command && (
                  <button
                    type="button"
                    disabled={saving}
                    onClick={() => void onSave(upsertCommand(workspace.commands, command, true))}
                    aria-label={t('aiCenter.routing.resetItem', 'Reset {{item}} to default', { item: item.name })}
                    title={t('aiCenter.routing.resetItem', 'Reset {{item}} to default', { item: item.name })}
                    className="flex h-8 w-8 items-center justify-center rounded-md disabled:opacity-40"
                    style={{ ...surface, ...muted }}
                  >
                    <RotateCcw size={13} />
                  </button>
                )}
              </form>
            </div>
          )
        })}
      </div>
    </div>
  )
}

function AdjustmentsPanel({
  workspace,
  saving,
  onSave,
  onOpen,
}: {
  workspace: RoutingWorkspace
  saving: boolean
  onSave: (commands: RoutingCommand[]) => Promise<void>
  onOpen: (path: string) => void
}) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const staleCount = workspace.status.filter((status) => status.staleReason).length
  if (workspace.commands.length === 0) return null
  return (
    <section className="rounded-xl" style={surface} data-testid="routing-adjustments">
      <button type="button" aria-expanded={open} onClick={() => setOpen(!open)} className="flex min-h-11 w-full items-center gap-2 px-3 text-left text-sm">
        {open ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
        <SlidersHorizontal size={15} style={{ color: 'var(--cp-accent)' }} />
        <span className="font-medium">{t('aiCenter.routing.adjustments', 'Routing adjustments')}</span>
        <span className="rounded-md px-2 py-0.5 text-xs" style={{ background: 'var(--cp-bg)', ...muted }}>{workspace.commands.length}</span>
        {staleCount > 0 && (
          <span className="inline-flex items-center gap-1 text-xs" style={{ color: 'var(--cp-warning)' }}>
            <AlertTriangle size={13} />{t('aiCenter.routing.staleCount', '{{count}} no longer apply', { count: staleCount })}
          </span>
        )}
      </button>
      {open && (
        <div className="flex flex-col gap-1 px-3 pb-3">
          {workspace.commands.map((command) => {
            const status = commandStatus(workspace, command)
            return (
              <div key={commandSubject(command)} className="flex flex-wrap items-center gap-2 rounded-lg px-2 py-1.5 text-xs" style={{ background: 'var(--cp-bg)' }}>
                <span className="min-w-0 flex-1 basis-60">
                  <CommandLabel command={command} onOpen={onOpen} />
                  {status?.staleReason ? (
                    <span className="mt-0.5 flex items-center gap-1" style={{ color: 'var(--cp-warning)' }}><AlertTriangle size={12} />{status.staleReason}</span>
                  ) : status ? (
                    <span className="mt-0.5 block" style={muted}>{t('aiCenter.routing.matchedItems', 'affects {{count}} items', { count: status.matchedItems })}</span>
                  ) : null}
                </span>
                <span className="font-mono tabular-nums">{command.kind === 'item_weight' ? `= ${formatWeight(command.weight)}` : `× ${formatWeight(command.factor)}`}</span>
                <button
                  type="button"
                  disabled={saving}
                  onClick={() => void onSave(upsertCommand(workspace.commands, command, true))}
                  aria-label={t('aiCenter.routing.removeAdjustment', 'Remove adjustment')}
                  title={t('aiCenter.routing.removeAdjustment', 'Remove adjustment')}
                  className="flex h-8 w-8 items-center justify-center rounded-md disabled:opacity-40"
                  style={{ color: 'var(--cp-danger)' }}
                >
                  <Trash2 size={13} />
                </button>
              </div>
            )
          })}
        </div>
      )}
    </section>
  )
}

function formatWeight(value: number): string {
  return Number.isInteger(value) ? value.toFixed(1) : String(Number(value.toFixed(4)))
}
