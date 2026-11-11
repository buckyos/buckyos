import { useState } from 'react'
import clsx from 'clsx'
import { ChevronRight, Folder, Plus } from 'lucide-react'
import { ago, t, type TextKey } from '../i18n'
import { sessionHref, useQuery } from '../lib'
import {
  dataModel,
  type ArtifactHead,
  type KnownRuntime,
  type KnownWorkspace,
  type RegistryEntry,
  type WorkspaceLocation,
} from '../model'
import { inputClass, mayHoldWorkspaces, runtimeAvailability } from '../runtimes'
import { Dialog, HomeShell } from '../shell'
import { ErrorBox } from '../ui'

const PREVIEW_COUNT = 5

const availabilityLabel: Record<KnownWorkspace['availability'], TextKey> = {
  available: 'workspaceAvailable',
  missing: 'workspaceMissing',
  runtime_unavailable: 'workspaceRuntimeUnavailable',
  permission_denied: 'workspacePermissionDenied',
  invalid_metadata: 'workspaceInvalidMetadata',
  conflict: 'workspaceConflict',
}

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

type WizardMode = 'import' | 'create'

/** Adding a workspace by hand: where (the runtime), what (an existing or a new directory), then the result of the check. */
function WorkspaceWizard({ onClose, refresh }: { onClose: () => void; refresh: () => Promise<unknown> }) {
  const { available } = useAvailableRuntimes()
  const defaultRuntime = available.find((r) => r.default)?.runtime_id ?? available[0]?.runtime_id ?? ''
  const [step, setStep] = useState<1 | 2 | 3>(1)
  const [runtime, setRuntime] = useState<string>()
  const [mode, setMode] = useState<WizardMode>('import')
  const [directory, setDirectory] = useState('')
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [url, setUrl] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>()
  const [created, setCreated] = useState<KnownWorkspace | null>(null)
  const chosen = runtime ?? defaultRuntime
  const chosenRuntime = available.find((r) => r.runtime_id === chosen)
  const submit = async () => {
    if (!directory.trim() || (mode === 'create' && !name.trim())) {
      setError(new Error(t('workspaceLocationRequired')))
      return
    }
    setStep(3)
    setBusy(true)
    setError(undefined)
    const location: WorkspaceLocation = { runtime_id: chosen, directory: directory.trim() }
    const operation_id = `web-${mode}-${crypto.randomUUID()}`
    try {
      const workspace = mode === 'create'
        ? await dataModel.createWorkspace({ operation_id, name: name.trim(), description: description.trim(), url: url.trim() || null, location })
        : await dataModel.importWorkspace({ operation_id, name: name.trim() || null, description: description.trim(), url: url.trim() || null, location })
      setCreated(workspace)
      await refresh()
    } catch (cause) {
      setError(cause)
    } finally {
      setBusy(false)
    }
  }
  const back = (to: 1 | 2) => { setError(undefined); setStep(to) }
  const footer = step === 1 ? <>
    <button type="button" className="tap flex-1 border border-line" onClick={onClose}>{t('cancel')}</button>
    <button type="submit" className="tap flex-1 bg-accent text-panel" disabled={!chosenRuntime}>{t('next')}</button>
  </> : step === 2 ? <>
    <button type="button" className="tap flex-1 border border-line" onClick={() => back(1)}>{t('back')}</button>
    <button type="submit" className="tap flex-1 bg-accent text-panel">{t('workspaceAddSubmit')}</button>
  </> : <>
    {!created && <button type="button" className="tap flex-1 border border-line" onClick={() => back(2)} disabled={busy}>{t('back')}</button>}
    <button type="button" className="tap flex-1 bg-accent text-panel" onClick={onClose} disabled={busy}>{t(created ? 'done' : 'close')}</button>
  </>
  return (
    <Dialog title={t('workspaceAdd')} busy={busy} onClose={onClose} testId="workspace-wizard" footer={footer}
      onSubmit={() => (step === 1 ? setStep(2) : step === 2 ? void submit() : undefined)}>
      <p className="mt-1 text-xs text-mute" data-testid="workspace-wizard-step">{t('wizardStep', { step, total: 3 })}</p>
      {step === 1 && <>
        <p className="my-3 text-sm text-mute">{t('workspaceRuntimeStepHint')}</p>
        {available.length ? <RuntimeSelect value={chosen} onChange={setRuntime} runtimes={available} />
          : <p className="mb-3 text-sm text-warn">{t('workspaceNoRuntime')}</p>}
      </>}
      {step === 2 && <>
        <p className="my-3 break-words text-sm text-mute">{t('workspaceOn', { runtime: chosenRuntime?.name ?? chosen })}</p>
        <div className="mb-3 flex gap-2" role="radiogroup">
          {(['import', 'create'] as const).map((m) => (
            <button key={m} type="button" role="radio" aria-checked={mode === m} onClick={() => setMode(m)} data-testid={`workspace-mode-${m}`}
              className={clsx('tap flex-1 border text-xs', mode === m ? 'border-accent text-accent' : 'border-line')}>
              {t(m === 'import' ? 'workspaceExistingDir' : 'workspaceNewDir')}
            </button>
          ))}
        </div>
        <p className="mb-3 text-xs text-mute">{t(mode === 'create' ? 'workspaceCreateHint' : 'workspaceImportHint')}</p>
        <label className="mb-3 block text-sm text-mute">
          {t('workspaceDirectory')}
          <input className={inputClass} value={directory} onChange={(event) => setDirectory(event.target.value)}
            placeholder={chosenRuntime?.grant.roots[0] ?? '/'} data-testid="workspace-directory" />
        </label>
        <label className="mb-3 block text-sm text-mute">
          {t('workspaceUrl')}
          <input className={inputClass} value={url} onChange={(event) => setUrl(event.target.value)} placeholder="https://github.com/org/project" data-testid="workspace-url" />
          <span className="mt-1 block text-xs">{t('workspaceUrlHint')}</span>
        </label>
        <label className="mb-3 block text-sm text-mute">
          {t('workspaceName')}
          <input className={inputClass} value={name} onChange={(event) => setName(event.target.value)} data-testid="workspace-name" />
        </label>
        <label className="mb-4 block text-sm text-mute">
          {t('workspaceDescription')}
          <input className={inputClass} value={description} onChange={(event) => setDescription(event.target.value)} />
        </label>
        <ErrorBox error={error} />
      </>}
      {step === 3 && <div className="my-3" data-testid="workspace-wizard-result">
        {busy && <p className="text-sm text-mute">{t('workspaceChecking')}</p>}
        {created && <>
          <p className="text-sm text-ok">{t('workspaceAdded')}</p>
          <dl className="mt-2 grid min-w-0 grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
            <dt className="text-mute">{t('workspaceName')}</dt><dd className="break-words">{created.name}</dd>
            <dt className="text-mute">{t('workspaceId')}</dt><dd className="break-all font-mono">{created.workspace_id}</dd>
            <dt className="text-mute">{t('workspaceDirectory')}</dt><dd className="break-all font-mono">{created.location.directory}</dd>
            <dt className="text-mute">{t('workspaceAccess')}</dt>
            <dd className={created.availability === 'available' ? 'text-ok' : 'text-warn'}>{t(availabilityLabel[created.availability])}</dd>
          </dl>
        </>}
        {!busy && !created && <>
          <p className="mb-2 text-sm text-warn">{t('workspaceAddFailed')}</p>
          <ErrorBox error={error} />
        </>}
      </div>}
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

const lastUsed = (workspace: KnownWorkspace, entries: RegistryEntry[]) =>
  entries.reduce((latest, entry) => (entry.workspace?.workspace_id === workspace.workspace_id ? Math.max(latest, entry.status.updated_at_ms) : latest), 0)
  || workspace.updated_at_ms

const runtimeOf = (workspace: KnownWorkspace, runtimes: KnownRuntime[]) => {
  const id = workspace.location.runtime_id
  return runtimes.find((r) => r.runtime_id === id || (r.connector.type === 'native' && (id === 'local' || id === 'container' || r.connector.aliases?.includes(id))))
}

function AddWorkspace({ refresh }: { refresh: () => Promise<unknown> }) {
  const [open, setOpen] = useState(false)
  return (
    <>
      <button className="tap border border-line text-xs" onClick={() => setOpen(true)} data-testid="workspace-add">
        <Plus size={14} />
        {t('workspaceAdd')}
      </button>
      {open && <WorkspaceWizard onClose={() => setOpen(false)} refresh={refresh} />}
    </>
  )
}

/** The home page's block: the workspaces used most recently. */
export function WorkspaceList({ entries = [] }: { entries?: RegistryEntry[] }) {
  const workspaces = useQuery(['home.workspaces'], () => dataModel.workspaces())
  const { all: runtimes } = useAvailableRuntimes()
  const records = [...(workspaces.data ?? [])].sort((a, b) => lastUsed(b, entries) - lastUsed(a, entries))
  return (
    <section className="card min-w-0 p-4" data-testid="workspaces">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-base font-semibold">{t('workspaces')}
          {workspaces.data && <span className="ml-1.5 text-sm font-normal text-mute">{records.length}</span>}
        </h2>
        <div className="flex items-center gap-2">
          <AddWorkspace refresh={() => workspaces.mutate()} />
          <a href="#/workspaces" className="flex min-h-11 items-center text-sm text-accent" data-testid="workspaces-all">
            {t('viewAll')}
            <ChevronRight size={16} />
          </a>
        </div>
      </div>
      <ErrorBox error={workspaces.error} />
      {records.length > 0 ? <ul className="divide-y divide-line">
        {records.slice(0, PREVIEW_COUNT).map((workspace) => (
          <li key={workspace.workspace_id} className="min-w-0 py-2" data-testid={`workspace-preview-${workspace.workspace_id}`}>
            <div className="flex items-center gap-2">
              <Folder size={16} className="shrink-0 text-accent" />
              <span className="min-w-0 flex-1 truncate text-sm font-medium">{workspace.name}</span>
              <span className="shrink-0 text-xs text-mute">{ago(lastUsed(workspace, entries))}</span>
            </div>
            <p className="truncate text-xs text-mute">
              {runtimeOf(workspace, runtimes)?.name ?? workspace.location.runtime_id} · <span className="font-mono">{workspace.location.directory}</span>
            </p>
            {workspace.url && <p className="truncate text-xs text-accent">{workspace.url}</p>}
            {workspace.description && <p className="truncate text-xs text-mute">{workspace.description}</p>}
          </li>
        ))}
      </ul> : !workspaces.error && <p className="py-2 text-sm text-mute">{t(workspaces.isLoading ? 'loading' : 'noWorkspaces')}</p>}
    </section>
  )
}

export default function WorkspacesPage() {
  const sessions = useQuery(['sessions.query'], () => dataModel.sessions())
  const workspaces = useQuery(['home.workspaces'], () => dataModel.workspaces())
  const artifacts = useQuery(['artifacts.list'], () => dataModel.artifacts())
  const { all: runtimes } = useAvailableRuntimes()
  const entries = sessions.data ?? []
  const records = [...(workspaces.data ?? [])].sort((a, b) => lastUsed(b, entries) - lastUsed(a, entries))
  return (
    <HomeShell title={t('workspaces')} testId="workspaces-page" extra={<AddWorkspace refresh={() => workspaces.mutate()} />}>
      <ErrorBox error={workspaces.error ?? artifacts.error ?? sessions.error} />
      <section className="card min-w-0 p-4">
        {records.length > 0 ? <ul className="divide-y divide-line">
          {records.map((workspace) => <WorkspaceCard key={workspace.workspace_id} workspace={workspace} entries={entries}
            artifacts={artifacts.data ?? []} runtimes={runtimes} refresh={() => workspaces.mutate()} />)}
        </ul> : !workspaces.error && <p className="py-2 text-sm text-mute">{t(workspaces.isLoading ? 'loading' : 'noWorkspaces')}</p>}
      </section>
    </HomeShell>
  )
}
