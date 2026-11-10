import { useState } from 'react'
import clsx from 'clsx'
import { Folder, Server } from 'lucide-react'
import { ago, t, type TextKey } from '../i18n'
import { sessionHref, useQuery } from '../lib'
import {
  dataModel,
  type ArtifactHead,
  type ComputerAccess,
  type ExtCheck,
  type KnownRuntime,
  type KnownWorkspace,
  type RegistryEntry,
  type RuntimeImpact,
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

const connectorLabel: Record<KnownRuntime['connector']['type'], TextKey> = {
  native: 'runtimeConnectorNative',
  node_executor: 'runtimeConnectorNode',
  ssh: 'runtimeConnectorSsh',
}

const computerLabel: Record<ComputerAccess, TextKey> = {
  none: 'computerNone',
  observe: 'computerObserve',
  control: 'computerControl',
}

const inputClass = 'mt-1 block min-h-11 w-full rounded-xl border border-line bg-bg px-3 text-base text-fg'

const WRITE_OPS = ['exec', 'write', 'edit', 'mkdir', 'rename', 'remove']
const isReadOnly = (runtime: KnownRuntime) => runtime.grant.ops.length > 0 && !runtime.grant.ops.some((op) => WRITE_OPS.includes(op))
const mayHoldWorkspaces = (runtime: KnownRuntime) =>
  runtime.lifecycle === 'active' && runtime.enabled && runtime.availability === 'available' && !isReadOnly(runtime) && runtime.grant.expires_at_ms == null

/** The runtimes a workspace can be placed on now. */
function useAvailableRuntimes() {
  const runtimes = useQuery(['runtimes.list'], () => dataModel.runtimes())
  return {
    all: runtimes.data ?? [],
    available: (runtimes.data ?? []).filter(mayHoldWorkspaces),
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
  onSubmit?: () => void
  children: React.ReactNode
  busy: boolean
  onClose: () => void
  label?: string
  testId?: string
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center bg-black/50 p-0 sm:items-center sm:p-4">
      <form role="dialog" aria-modal="true" aria-label={title} data-testid={testId}
        className="home max-h-[92dvh] w-full overflow-y-auto rounded-t-2xl border border-line bg-panel p-5 sm:max-w-md sm:rounded-2xl"
        onSubmit={(event) => { event.preventDefault(); onSubmit?.() }}>
        <h3 className="text-base font-semibold">{title}</h3>
        {children}
        <div className="flex gap-2">
          <button type="button" className="tap flex-1 border border-line" onClick={onClose} disabled={busy}>{t(onSubmit ? 'cancel' : 'close')}</button>
          {onSubmit && <button type="submit" className="tap flex-1 bg-accent text-panel" disabled={busy}>{label}</button>}
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
          placeholder={available.find((r) => r.runtime_id === chosen)?.grant.roots[0] ?? '/'} data-testid="workspace-directory" />
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

function extToolsLine(ext: ExtCheck): string {
  switch (ext.status) {
    case 'ok': return t('runtimeExtOk')
    case 'partial': return t('runtimeExtPartial', { tools: ext.missing.join(', ') })
    case 'none': return t('runtimeExtNone')
    case 'system': return t('runtimeExtSystem', { tools: ext.system.join(', ') })
  }
}

/** What the node installed for this runtime. Read-only: packages are chosen for the whole Zone, not per Agent. */
function RuntimeExtPackages({ runtimeId }: { runtimeId: string }) {
  const report = useQuery(['runtimes.ext_tools', runtimeId], () => dataModel.runtimeExtTools(runtimeId))
  const packages = Object.entries(report.data?.packages ?? {})
  if (packages.length === 0) return null
  const names = packages.map(([id, install]) => `${id} ${install.version}`).join(', ')
  return (
    <p className="mt-1 break-words text-xs text-mute" data-testid="runtime-ext-packages">
      {t('runtimeExtPackages', { packages: names })} · {t('runtimeExtManagedByNode')}
    </p>
  )
}

function RuntimeCard({ runtime, chosen, configuredDefault, refresh }: {
  runtime: KnownRuntime
  /** The owner's setting for this runtime on the agent, when there is one. */
  chosen: { enabled: boolean } | null
  /** The owner's default, also while it is switched off. */
  configuredDefault: boolean
  refresh: () => Promise<unknown>
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>()
  const [impact, setImpact] = useState<RuntimeImpact | null>(null)
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
  const toggle = () => run(async () => {
    if (!chosen?.enabled) return dataModel.setRuntimeSetting({ runtime_id: runtime.runtime_id, enabled: true })
    const found = await dataModel.runtimeImpact(runtime.runtime_id)
    if (found.blocking) setImpact(found)
    else await dataModel.setRuntimeSetting({ runtime_id: runtime.runtime_id, enabled: false })
  })
  const isPrivate = runtime.connector.type === 'native'
  const retired = runtime.lifecycle === 'retired'
  const { caps, grant, usage } = runtime
  const check = caps.baseline_check
  const devHelpers = Object.entries(check?.resolved ?? {}).filter(([, command]) => command.provided_by === 'dev').map(([name]) => name)
  const extMissing = check?.ext?.missing ?? []
  const standardMissing = check?.missing.filter((tool) => !extMissing.includes(tool)) ?? []
  const toolProblem = check?.error
    ? t('runtimeToolCheckFailed', { error: check.error })
    : [
        standardMissing.length > 0 && t('runtimeToolsMissing', { tools: standardMissing.join(', ') }),
        check?.unmanaged?.length && t('runtimeToolsUnmanaged', { tools: check.unmanaged.join(', ') }),
      ].filter(Boolean).join(' · ')
  return (
    <li className={clsx('min-w-0 py-3 first:pt-1 last:pb-0', retired && 'opacity-60')} data-testid={`runtime-${runtime.runtime_id}`}>
      <div className="flex items-start gap-2">
        <Server size={18} className="mt-0.5 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <h3 className="break-words text-sm font-semibold">{runtime.name}
            <span className="ml-1.5 font-mono text-xs font-normal text-mute">{runtime.runtime_id}</span>
          </h3>
          {runtime.desc && <p className="mt-1 break-words text-sm text-mute">{runtime.desc}</p>}
        </div>
      </div>
      <div className="my-2 flex flex-wrap gap-x-3 gap-y-1 text-xs">
        <span>{t(connectorLabel[runtime.connector.type])}</span>
        {isPrivate && <span className="text-accent">{t('runtimePrivate')}</span>}
        <span className={clsx(runtime.availability === 'available' ? 'text-ok' : 'text-warn')}>
          {t(retired ? 'runtimeRetired' : runtimeAvailability[runtime.availability])}
        </span>
        {(runtime.default || configuredDefault) && <span className="text-accent" data-testid="runtime-default">{t(runtime.default ? 'runtimeDefault' : 'runtimeDefaultOff')}</span>}
        {runtime.builtin && <span>{t('runtimeBuiltin')}</span>}
        <span data-testid="runtime-computer">{t('runtimeComputer')}: {t(computerLabel[grant.computer ?? 'none'])}</span>
        <span data-testid="runtime-baseline">{t(caps.baseline === 'extended' ? 'runtimeBaselineExtended' : 'runtimeBaselineStandard')}</span>
        {caps.os && <span className="font-mono">{caps.os}{caps.shell ? ` · ${caps.shell}` : ''}{caps.identity_mode ? ` · ${caps.identity_mode}` : ''}</span>}
        {caps.desktop && <span>{t('runtimeDesktop')}</span>}
        {!mayHoldWorkspaces(runtime) && <span>{t('runtimeNoWorkspaces')}</span>}
        {isReadOnly(runtime) && <span>{t('runtimeReadOnly')}</span>}
        {grant.expires_at_ms != null && <span className="text-warn">{t('runtimeExpires', { time: new Date(grant.expires_at_ms).toLocaleString() })}</span>}
      </div>
      {grant.roots.length > 0 && <dl className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
        <dt className="text-mute">{t('runtimeRoots')}</dt>
        <dd className="break-all font-mono">{grant.roots.join(' · ')}</dd>
      </dl>}
      {(usage.sessions_total > 0 || usage.workspaces > 0) &&
        <p className="mt-1 text-xs text-mute" data-testid="runtime-usage">
          {t('runtimeUsage', { sessions: String(usage.sessions_total), workspaces: String(usage.workspaces) })}
          {usage.last_session_id && <> · <a className="font-mono text-accent" href={sessionHref(usage.last_session_id)}>{usage.last_session_id}</a></>}
        </p>}
      {runtime.last_error && runtime.availability !== 'available' &&
        <p className="mt-2 break-words text-xs text-warn">{runtime.last_error}</p>}
      {caps.desktop_error && <p className="mt-2 break-words text-xs text-warn" data-testid="runtime-desktop-error">{caps.desktop_error}</p>}
      {toolProblem &&
        <p className="mt-2 break-words text-xs text-warn" data-testid="runtime-tool-check">{toolProblem}</p>}
      {check?.ext &&
        <p className={clsx('mt-1 break-words text-xs', check.missing.some((tool) => extMissing.includes(tool)) ? 'text-warn' : 'text-mute')} data-testid="runtime-ext">{extToolsLine(check.ext)}</p>}
      {!retired && runtime.availability === 'available' && <RuntimeExtPackages runtimeId={runtime.runtime_id} />}
      {devHelpers.length > 0 &&
        <p className="mt-1 break-words text-xs text-mute" data-testid="runtime-dev-helpers">{t('runtimeDevHelpers', { tools: devHelpers.join(', ') })}</p>}
      {caps.tools_check && caps.tools_check.present.length > 0 &&
        <p className="mt-1 break-words text-xs text-mute">{t('runtimeCommandsVerified', { tools: caps.tools_check.present.join(', ') })}</p>}
      {!impact && <ErrorBox error={error} />}
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <button className="tap border border-line text-xs" disabled={busy} onClick={() => void run(() => dataModel.checkRuntime(runtime.runtime_id))}>{t('runtimeCheck')}</button>
        {chosen && !isPrivate && <button className="tap border border-line text-xs" disabled={busy} data-testid="runtime-toggle"
          onClick={() => void toggle()}>
          {t(chosen.enabled ? 'runtimeDisable' : 'runtimeEnable')}
        </button>}
        {!runtime.default && !retired && (isPrivate || chosen?.enabled) && <button className="tap border border-line text-xs" disabled={busy} data-testid="runtime-make-default"
          onClick={() => void run(() => dataModel.setRuntimeSetting({ runtime_id: runtime.runtime_id, default: true }))}>
          {t('runtimeMakeDefault')}
        </button>}
      </div>
      <p className="mt-2 text-xs text-mute" data-testid="runtime-config-hint">{t('runtimeConfigHint')}</p>
      {impact && <Dialog title={t('runtimeInUseTitle', { name: runtime.name })} busy={busy} testId="runtime-impact"
        onClose={() => setImpact(null)}>
        <p className="my-3 text-sm text-mute">{t('runtimeInUseHint')}</p>
        {impact.running_sessions.length > 0 && <div className="mb-3 text-sm" data-testid="runtime-impact-sessions">
          <div className="text-xs text-mute">{t('runtimeImpactSessions', { count: impact.running_sessions.length })}</div>
          {impact.active_sessions.filter((session) => impact.running_sessions.includes(session.session_id)).map((session) => <a key={session.session_id} href={sessionHref(session.session_id)}
            className="block break-words text-accent">{session.title || session.session_id} · {session.run_state}</a>)}
        </div>}
        <ErrorBox error={error} />
      </Dialog>}
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
      <p className="mb-3 text-xs text-mute">{t('runtimeModelHint')}</p>
      <ErrorBox error={runtimes.error ?? settings.error} />
      {records.length > 0 ? <ul className="divide-y divide-line">
        {records.map((runtime) => {
          const settingId = runtime.connector.type === 'native' ? (runtime.connector.aliases?.[0] ?? runtime.runtime_id) : runtime.runtime_id
          return <RuntimeCard key={runtime.runtime_id} runtime={runtime} chosen={settings.data?.runtimes[settingId] ?? null}
            configuredDefault={settings.data?.configured_default_runtime_id === settingId} refresh={refresh} />
        })}
      </ul> : !runtimes.error && <p className="py-2 text-sm text-mute">{t(runtimes.isLoading ? 'loading' : 'noRuntimes')}</p>}
    </section>
  )
}
