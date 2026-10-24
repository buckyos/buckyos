import { useState } from 'react'
import clsx from 'clsx'
import { Folder, Server } from 'lucide-react'
import { ago, t, type TextKey } from '../i18n'
import { sessionHref, useQuery } from '../lib'
import {
  dataModel,
  type ArtifactHead,
  type ComputerAccess,
  type KnownRuntime,
  type KnownWorkspace,
  type RegistryEntry,
  type WorkspaceLocation,
} from '../model'
import { ErrorBox } from '../ui'

const availabilityLabel: Record<KnownWorkspace['availability'], TextKey> = {
  available: 'workspaceAvailable',
  missing: 'workspaceMissing',
  runtime_unavailable: 'workspaceRuntimeUnavailable',
  permission_denied: 'workspacePermissionDenied',
  invalid_metadata: 'workspaceInvalidMetadata',
  conflict: 'workspaceConflict',
}

const runtimeKindLabel: Record<KnownRuntime['kind'], TextKey> = {
  container: 'runtimeKindContainer',
  host_native: 'runtimeKindHost',
  remote_node: 'runtimeKindNode',
  remote_ssh: 'runtimeKindSsh',
}

const computerLabel: Record<ComputerAccess, TextKey> = {
  none: 'computerNone',
  observe: 'computerObserve',
  control: 'computerControl',
}

const inputClass = 'mt-1 block min-h-11 w-full rounded-xl border border-line bg-bg px-3 text-base text-fg'

/** The runtimes a workspace can be placed on now. */
function useAvailableRuntimes() {
  const runtimes = useQuery(['runtimes.list'], () => dataModel.runtimes())
  return {
    all: runtimes.data ?? [],
    available: (runtimes.data ?? []).filter((runtime) => runtime.availability === 'available'),
    refresh: () => runtimes.mutate(),
  }
}

function RuntimeSelect({ value, onChange, runtimes }: { value: string; onChange: (id: string) => void; runtimes: KnownRuntime[] }) {
  return (
    <label className="mb-3 block text-sm text-mute">
      {t('workspaceRuntime')}
      <select className={inputClass} value={value} onChange={(event) => onChange(event.target.value)} data-testid="workspace-runtime-select">
        {runtimes.map((runtime) => (
          <option key={runtime.runtime_id} value={runtime.runtime_id}>
            {runtime.name} · {runtime.runtime_id}{runtime.default ? ` · ${t('runtimeDefault')}` : ''}
          </option>
        ))}
      </select>
    </label>
  )
}

function Dialog({ title, onSubmit, children, busy, onClose, label, testId }: {
  title: string
  onSubmit: () => void
  children: React.ReactNode
  busy: boolean
  onClose: () => void
  label: string
  testId?: string
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center bg-black/50 p-0 sm:items-center sm:p-4">
      <form role="dialog" aria-modal="true" aria-label={title} data-testid={testId}
        className="home max-h-[92dvh] w-full overflow-y-auto rounded-t-2xl border border-line bg-panel p-5 sm:max-w-md sm:rounded-2xl"
        onSubmit={(event) => { event.preventDefault(); onSubmit() }}>
        <h3 className="text-base font-semibold">{title}</h3>
        {children}
        <div className="flex gap-2">
          <button type="button" className="tap flex-1 border border-line" onClick={onClose} disabled={busy}>{t('cancel')}</button>
          <button type="submit" className="tap flex-1 bg-accent text-panel" disabled={busy}>{label}</button>
        </div>
      </form>
    </div>
  )
}

function RelocateDialog({ workspace, onClose, refresh }: {
  workspace: KnownWorkspace
  onClose: () => void
  refresh: () => Promise<unknown>
}) {
  const { available } = useAvailableRuntimes()
  const [runtime, setRuntime] = useState(workspace.location.runtime_id)
  const [directory, setDirectory] = useState(workspace.location.directory)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>()
  const choices = available.some((r) => r.runtime_id === runtime) ? available : [...available, { runtime_id: runtime, name: runtime, default: false } as KnownRuntime]
  const save = async () => {
    if (!runtime.trim() || !directory.trim()) {
      setError(new Error(t('workspaceLocationRequired')))
      return
    }
    setBusy(true)
    setError(undefined)
    try {
      await dataModel.relocateWorkspace(workspace, { runtime_id: runtime.trim(), directory: directory.trim() })
      await refresh()
      onClose()
    } catch (cause) {
      setError(cause)
      await refresh()
    } finally {
      setBusy(false)
    }
  }
  return (
    <Dialog title={t('workspaceRelocate')} onSubmit={() => void save()} busy={busy} onClose={onClose} label={t('save')}>
      <p className="my-3 break-words text-sm">{workspace.name}</p>
      <p className="mb-4 text-sm text-mute">{t('workspaceRelocateHint')}</p>
      <RuntimeSelect value={runtime} onChange={setRuntime} runtimes={choices} />
      <label className="mb-4 block text-sm text-mute">
        {t('workspaceDirectory')}
        <input className={inputClass} value={directory} onChange={(event) => setDirectory(event.target.value)} />
      </label>
      <ErrorBox error={error} />
    </Dialog>
  )
}

function WorkspaceForm({ mode, onClose, refresh }: {
  mode: 'create' | 'import'
  onClose: () => void
  refresh: () => Promise<unknown>
}) {
  const { available } = useAvailableRuntimes()
  const defaultRuntime = available.find((r) => r.default)?.runtime_id ?? available[0]?.runtime_id ?? 'container'
  const [runtime, setRuntime] = useState<string>()
  const [directory, setDirectory] = useState('')
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [url, setUrl] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>()
  const chosen = runtime ?? defaultRuntime
  const submit = async () => {
    if (!directory.trim() || (mode === 'create' && !name.trim())) {
      setError(new Error(t('workspaceLocationRequired')))
      return
    }
    setBusy(true)
    setError(undefined)
    const location: WorkspaceLocation = { runtime_id: chosen, directory: directory.trim() }
    const operation_id = `web-${mode}-${crypto.randomUUID()}`
    try {
      if (mode === 'create') {
        await dataModel.createWorkspace({ operation_id, name: name.trim(), description: description.trim(), url: url.trim() || null, location })
      } else {
        await dataModel.importWorkspace({ operation_id, name: name.trim() || null, description: description.trim(), url: url.trim() || null, location })
      }
      await refresh()
      onClose()
    } catch (cause) {
      setError(cause)
    } finally {
      setBusy(false)
    }
  }
  return (
    <Dialog title={t(mode === 'create' ? 'workspaceCreate' : 'workspaceImport')} onSubmit={() => void submit()} busy={busy}
      onClose={onClose} label={t('save')} testId={`workspace-${mode}-form`}>
      <p className="my-3 text-sm text-mute">{t(mode === 'create' ? 'workspaceCreateHint' : 'workspaceImportHint')}</p>
      <RuntimeSelect value={chosen} onChange={setRuntime} runtimes={available} />
      <label className="mb-3 block text-sm text-mute">
        {t('workspaceDirectory')}
        <input className={inputClass} value={directory} onChange={(event) => setDirectory(event.target.value)}
          placeholder={available.find((r) => r.runtime_id === chosen)?.roots[0] ?? '/'} data-testid="workspace-directory" />
      </label>
      <label className="mb-3 block text-sm text-mute">
        {t('workspaceName')}
        <input className={inputClass} value={name} onChange={(event) => setName(event.target.value)} data-testid="workspace-name" />
      </label>
      <label className="mb-3 block text-sm text-mute">
        {t('workspaceDescription')}
        <input className={inputClass} value={description} onChange={(event) => setDescription(event.target.value)} />
      </label>
      <label className="mb-4 block text-sm text-mute">
        {t('workspaceUrl')}
        <input className={inputClass} value={url} onChange={(event) => setUrl(event.target.value)} placeholder="https://github.com/org/project" data-testid="workspace-url" />
        <span className="mt-1 block text-xs">{t('workspaceUrlHint')}</span>
      </label>
      <ErrorBox error={error} />
    </Dialog>
  )
}

function UrlDialog({ workspace, onClose, refresh }: { workspace: KnownWorkspace; onClose: () => void; refresh: () => Promise<unknown> }) {
  const [url, setUrl] = useState(workspace.url ?? '')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>()
  const save = async () => {
    setBusy(true)
    setError(undefined)
    try {
      await dataModel.setWorkspaceUrl(workspace, url.trim())
      await refresh()
      onClose()
    } catch (cause) {
      setError(cause)
      await refresh()
    } finally {
      setBusy(false)
    }
  }
  return (
    <Dialog title={t('workspaceSetUrl')} onSubmit={() => void save()} busy={busy} onClose={onClose} label={t('save')}>
      <p className="my-3 break-words text-sm">{workspace.name}</p>
      <label className="mb-4 block text-sm text-mute">
        {t('workspaceUrl')}
        <input className={inputClass} value={url} onChange={(event) => setUrl(event.target.value)} placeholder="https://github.com/org/project" />
        <span className="mt-1 block text-xs">{t('workspaceUrlHint')}</span>
      </label>
      <ErrorBox error={error} />
    </Dialog>
  )
}

function WorkspaceCard({ workspace, entries, artifacts, runtimes, refresh }: {
  workspace: KnownWorkspace
  entries: RegistryEntry[]
  artifacts: ArtifactHead[]
  runtimes: KnownRuntime[]
  refresh: () => Promise<unknown>
}) {
  const [editing, setEditing] = useState<'relocate' | 'url' | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>()
  const sessions = entries.filter((entry) => entry.workspace?.workspace_id === workspace.workspace_id)
    .sort((a, b) => b.status.updated_at_ms - a.status.updated_at_ms)
  const relatedArtifacts = artifacts.filter((artifact) => artifact.workspace?.workspace_id === workspace.workspace_id)
    .sort((a, b) => b.updated_at_ms - a.updated_at_ms).slice(0, 3)
  const unfinished = sessions.filter((entry) => entry.status.run_state !== 'finished').length
  const runtimeId = workspace.location.runtime_id === 'local' ? 'container' : workspace.location.runtime_id
  const runtime = runtimes.find((r) => r.runtime_id === runtimeId)
  const check = async () => {
    setBusy(true)
    setError(undefined)
    try {
      await dataModel.checkWorkspace(workspace.workspace_id)
      await refresh()
    } catch (cause) {
      setError(cause)
      await refresh()
    } finally {
      setBusy(false)
    }
  }
  return (
    <li className="min-w-0 py-4 first:pt-1 last:pb-0" data-testid={`workspace-${workspace.workspace_id}`}>
      <div className="flex items-start gap-2">
        <Folder size={18} className="mt-0.5 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <h3 className="break-words text-sm font-semibold">{workspace.name}</h3>
          {workspace.description && <p className="mt-1 whitespace-pre-line break-words text-sm text-mute">{workspace.description}</p>}
          {workspace.url && <a className="mt-1 block break-all text-sm text-accent" href={workspace.url} target="_blank" rel="noreferrer" data-testid="workspace-url-link">{workspace.url}</a>}
        </div>
      </div>
      <div className="my-2 flex flex-wrap gap-x-3 gap-y-1 text-xs">
        <span>{t(workspace.usage === 'private' ? 'workspacePrivate' : 'workspaceCollaborative')}</span>
        <span>{t(workspace.lifecycle === 'archived' ? 'workspaceArchived' : 'workspaceActive')}</span>
        <span className={clsx(workspace.availability === 'available' ? 'text-ok' : 'text-warn')}>
          {t(availabilityLabel[workspace.availability])}
        </span>
        {unfinished > 0 && <span className="text-accent">{t('workspaceBusy', { count: unfinished })}</span>}
      </div>
      <dl className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
        <dt className="text-mute">{t('workspaceId')}</dt><dd className="break-all font-mono">{workspace.workspace_id}</dd>
        <dt className="text-mute">{t('workspaceRuntime')}</dt>
        <dd className="break-all">
          {runtime ? `${runtime.name} · ` : ''}{workspace.location.runtime_id}
          {runtime && <span className={clsx('ml-1.5', runtime.availability === 'available' ? 'text-ok' : 'text-warn')}>
            {t(runtimeAvailability[runtime.availability])}
          </span>}
        </dd>
        <dt className="text-mute">{t('workspaceDirectory')}</dt><dd className="break-all font-mono">{workspace.location.directory}</dd>
      </dl>
      <p className="mt-2 text-xs text-mute" title={workspace.checked_at_ms ? new Date(workspace.checked_at_ms).toLocaleString() : undefined}>
        {workspace.checked_at_ms ? t('workspaceChecked', { time: ago(workspace.checked_at_ms) }) : t('workspaceUnchecked')}
      </p>
      {workspace.last_error && <p className="mt-2 break-words text-xs text-warn">{workspace.last_error}</p>}
      {workspace.conflict && <p className="mt-2 break-all text-xs text-warn">
        {workspace.conflict.message} · {workspace.conflict.candidate.runtime_id}: {workspace.conflict.candidate.directory}
      </p>}
      {sessions.length > 0 && <div className="mt-2 text-xs">
        <div className="text-mute">{t('workspaceRecentWork')}</div>
        {sessions.slice(0, 3).map((entry) => <a key={entry.session_id} href={sessionHref(entry.session_id)}
          className="block min-h-11 content-center break-words text-accent">{entry.objective || entry.session_id}</a>)}
      </div>}
      {workspace.source_session && !sessions.some((entry) => entry.session_id === workspace.source_session) &&
        <a className="mt-2 block min-h-11 content-center break-all text-xs text-accent" href={sessionHref(workspace.source_session)}>
          {t('workspaceSource')} · {workspace.source_session}
        </a>}
      {relatedArtifacts.length > 0 && <div className="mt-2 text-xs">
        <div className="text-mute">{t('workspaceRecentArtifacts')}</div>
        {relatedArtifacts.map((artifact) => <a key={artifact.aid} className="block min-h-11 content-center break-all text-accent"
          href={sessions.find((entry) => entry.artifact_id === artifact.aid)
            ? sessionHref(sessions.find((entry) => entry.artifact_id === artifact.aid)!.session_id) : '#/agent'}>{artifact.aid}</a>)}
      </div>}
      <ErrorBox error={error} />
      <div className="mt-2 flex flex-wrap gap-2">
        <button className="tap border border-line text-xs" onClick={() => void check()} disabled={busy}>{t('workspaceCheck')}</button>
        <button className="tap border border-line text-xs" onClick={() => setEditing('url')} disabled={busy}>{t('workspaceSetUrl')}</button>
        <button className="tap border border-line text-xs" onClick={() => setEditing('relocate')} disabled={busy}>{t('workspaceRelocate')}</button>
      </div>
      {editing === 'relocate' && <RelocateDialog workspace={workspace} onClose={() => setEditing(null)} refresh={refresh} />}
      {editing === 'url' && <UrlDialog workspace={workspace} onClose={() => setEditing(null)} refresh={refresh} />}
    </li>
  )
}

export function WorkspaceList({ entries = [] }: { entries?: RegistryEntry[] }) {
  const workspaces = useQuery(['home.workspaces'], () => dataModel.workspaces())
  const artifacts = useQuery(['artifacts.list'], () => dataModel.artifacts())
  const { all: runtimes } = useAvailableRuntimes()
  const [form, setForm] = useState<'create' | 'import' | null>(null)
  const records = [...(workspaces.data ?? [])].sort((a, b) => b.updated_at_ms - a.updated_at_ms)
  return (
    <section className="card min-w-0 p-4" data-testid="known-workspaces">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-base font-semibold">{t('knownWorkspaces')}
          {workspaces.data && <span className="ml-1.5 text-sm font-normal text-mute">{records.length}</span>}
        </h2>
        <div className="flex gap-2">
          <button className="tap border border-line text-xs" onClick={() => setForm('create')} data-testid="workspace-create">{t('workspaceCreate')}</button>
          <button className="tap border border-line text-xs" onClick={() => setForm('import')} data-testid="workspace-import">{t('workspaceImport')}</button>
        </div>
      </div>
      <ErrorBox error={workspaces.error ?? artifacts.error} />
      {records.length > 0 ? <ul className="divide-y divide-line">
        {records.map((workspace) => <WorkspaceCard key={workspace.workspace_id} workspace={workspace} entries={entries}
          artifacts={artifacts.data ?? []} runtimes={runtimes} refresh={() => workspaces.mutate()} />)}
      </ul> : !workspaces.error && <p className="py-2 text-sm text-mute">{t(workspaces.isLoading ? 'loading' : 'noWorkspaces')}</p>}
      {form && <WorkspaceForm mode={form} onClose={() => setForm(null)} refresh={() => workspaces.mutate()} />}
    </section>
  )
}

const runtimeAvailability: Record<KnownRuntime['availability'], TextKey> = {
  available: 'runtimeAvailable',
  runtime_unavailable: 'runtimeUnavailable',
  permission_denied: 'runtimePermissionDenied',
}

function RuntimeCard({ runtime, chosen, refresh }: {
  runtime: KnownRuntime
  /** The owner's setting for this runtime on the agent, when there is one. */
  chosen: { enabled: boolean; computer: ComputerAccess } | null
  refresh: () => Promise<unknown>
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>()
  const run = async (action: () => Promise<unknown>) => {
    setBusy(true)
    setError(undefined)
    try {
      await action()
      await refresh()
    } catch (cause) {
      setError(cause)
      await refresh()
    } finally {
      setBusy(false)
    }
  }
  const capabilities = (runtime.capabilities ?? {}) as { os?: string; identity_mode?: string; desktop?: { ui?: { ready?: boolean; state?: string } } }
  const isContainer = runtime.kind === 'container'
  return (
    <li className="min-w-0 py-3 first:pt-1 last:pb-0" data-testid={`runtime-${runtime.runtime_id}`}>
      <div className="flex items-start gap-2">
        <Server size={18} className="mt-0.5 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <h3 className="break-words text-sm font-semibold">{runtime.name}
            <span className="ml-1.5 font-mono text-xs font-normal text-mute">{runtime.runtime_id}</span>
          </h3>
          {runtime.description && <p className="mt-1 break-words text-sm text-mute">{runtime.description}</p>}
        </div>
      </div>
      <div className="my-2 flex flex-wrap gap-x-3 gap-y-1 text-xs">
        <span>{t(runtimeKindLabel[runtime.kind])}</span>
        <span className={clsx(runtime.availability === 'available' ? 'text-ok' : 'text-warn')}>
          {t(runtimeAvailability[runtime.availability])}
        </span>
        {runtime.default && <span className="text-accent" data-testid="runtime-default">{t('runtimeDefault')}</span>}
        {runtime.builtin && <span>{t('runtimeBuiltin')}</span>}
        {!isContainer && <span>{t('runtimeComputer')}: {t(computerLabel[runtime.computer ?? 'none'])}</span>}
        {capabilities.os && <span className="font-mono">{capabilities.os}{capabilities.identity_mode ? ` · ${capabilities.identity_mode}` : ''}</span>}
        {capabilities.desktop?.ui && <span className={clsx(capabilities.desktop.ui.ready ? 'text-ok' : 'text-warn')}>
          desktop: {capabilities.desktop.ui.state ?? (capabilities.desktop.ui.ready ? 'ready' : 'not ready')}
        </span>}
      </div>
      {runtime.roots.length > 0 && <dl className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
        <dt className="text-mute">{t('runtimeRoots')}</dt>
        <dd className="break-all font-mono">{runtime.roots.join(' · ')}</dd>
      </dl>}
      {runtime.last_error && runtime.availability !== 'available' &&
        <p className="mt-2 break-words text-xs text-warn">{runtime.last_error}</p>}
      <ErrorBox error={error} />
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <button className="tap border border-line text-xs" disabled={busy} onClick={() => void run(() => dataModel.checkRuntime(runtime.runtime_id))}>{t('runtimeCheck')}</button>
        {chosen && <button className="tap border border-line text-xs" disabled={busy} data-testid="runtime-toggle"
          onClick={() => void run(() => dataModel.setRuntimeSetting({ runtime_id: runtime.runtime_id, enabled: !chosen.enabled }))}>
          {t(chosen.enabled ? 'runtimeDisable' : 'runtimeEnable')}
        </button>}
        {!runtime.default && (isContainer || chosen?.enabled) && <button className="tap border border-line text-xs" disabled={busy} data-testid="runtime-make-default"
          onClick={() => void run(() => dataModel.setRuntimeSetting({ runtime_id: runtime.runtime_id, default: true }))}>
          {t('runtimeMakeDefault')}
        </button>}
        {chosen && <label className="text-xs text-mute">
          {t('runtimeComputer')}
          <select className="ml-1 rounded-lg border border-line bg-bg px-2 py-1 text-xs text-fg" value={chosen.computer} disabled={busy}
            onChange={(event) => void run(() => dataModel.setRuntimeSetting({ runtime_id: runtime.runtime_id, computer: event.target.value as ComputerAccess }))}>
            {(['none', 'observe', 'control'] as ComputerAccess[]).map((value) => <option key={value} value={value}>{t(computerLabel[value])}</option>)}
          </select>
        </label>}
      </div>
    </li>
  )
}

export function RuntimeList() {
  const runtimes = useQuery(['runtimes.list'], () => dataModel.runtimes())
  const settings = useQuery(['runtimes.settings'], () => dataModel.runtimeSettings())
  const records = runtimes.data ?? []
  const refresh = () => Promise.all([runtimes.mutate(), settings.mutate()])
  return (
    <section className="card min-w-0 p-4" data-testid="known-runtimes">
      <h2 className="mb-1 text-base font-semibold">{t('runtimes')}
        {runtimes.data && <span className="ml-1.5 text-sm font-normal text-mute">{records.length}</span>}
      </h2>
      <p className="mb-3 text-xs text-mute">{t('runtimeManagedHint')}</p>
      <ErrorBox error={runtimes.error ?? settings.error} />
      {records.length > 0 ? <ul className="divide-y divide-line">
        {records.map((runtime) => <RuntimeCard key={runtime.runtime_id} runtime={runtime}
          chosen={settings.data?.runtimes[runtime.runtime_id] ?? null} refresh={refresh} />)}
      </ul> : !runtimes.error && <p className="py-2 text-sm text-mute">{t(runtimes.isLoading ? 'loading' : 'noRuntimes')}</p>}
    </section>
  )
}
