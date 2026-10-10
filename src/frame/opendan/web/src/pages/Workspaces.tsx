import { useState } from 'react'
import clsx from 'clsx'
import { Folder } from 'lucide-react'
import { ago, t, type TextKey } from '../i18n'
import { sessionHref, useQuery } from '../lib'
import { dataModel, type ArtifactHead, type KnownWorkspace, type RegistryEntry } from '../model'
import { ErrorBox } from '../ui'

const availabilityLabel: Record<KnownWorkspace['availability'], TextKey> = {
  available: 'workspaceAvailable',
  missing: 'workspaceMissing',
  runtime_unavailable: 'workspaceRuntimeUnavailable',
  permission_denied: 'workspacePermissionDenied',
  invalid_metadata: 'workspaceInvalidMetadata',
  conflict: 'workspaceConflict',
}

function RelocateDialog({ workspace, onClose, refresh }: {
  workspace: KnownWorkspace
  onClose: () => void
  refresh: () => Promise<unknown>
}) {
  const [runtime, setRuntime] = useState(workspace.location.runtime_id)
  const [directory, setDirectory] = useState(workspace.location.directory)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>()
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
    <div className="fixed inset-0 z-50 flex items-end justify-center bg-black/50 p-0 sm:items-center sm:p-4">
      <form role="dialog" aria-modal="true" aria-label={t('workspaceRelocate')}
        className="home max-h-[92dvh] w-full overflow-y-auto rounded-t-2xl border border-line bg-panel p-5 sm:max-w-md sm:rounded-2xl"
        onSubmit={(event) => { event.preventDefault(); void save() }}>
        <h3 className="text-base font-semibold">{t('workspaceRelocate')}</h3>
        <p className="my-3 break-words text-sm">{workspace.name}</p>
        <p className="mb-4 text-sm text-mute">{t('workspaceRelocateHint')}</p>
        <label className="mb-3 block text-sm text-mute">
          {t('workspaceRuntime')}
          <input className="mt-1 block min-h-11 w-full rounded-xl border border-line bg-bg px-3 text-base text-fg"
            value={runtime} onChange={(event) => setRuntime(event.target.value)} />
        </label>
        <label className="mb-4 block text-sm text-mute">
          {t('workspaceDirectory')}
          <input className="mt-1 block min-h-11 w-full rounded-xl border border-line bg-bg px-3 text-base text-fg"
            value={directory} onChange={(event) => setDirectory(event.target.value)} />
        </label>
        <ErrorBox error={error} />
        <div className="flex gap-2">
          <button type="button" className="tap flex-1 border border-line" onClick={onClose} disabled={busy}>{t('cancel')}</button>
          <button type="submit" className="tap flex-1 bg-accent text-panel" disabled={busy}>{t('save')}</button>
        </div>
      </form>
    </div>
  )
}

function WorkspaceCard({ workspace, entries, artifacts, refresh }: {
  workspace: KnownWorkspace
  entries: RegistryEntry[]
  artifacts: ArtifactHead[]
  refresh: () => Promise<unknown>
}) {
  const [editing, setEditing] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>()
  const sessions = entries.filter((entry) => entry.workspace?.workspace_id === workspace.workspace_id)
    .sort((a, b) => b.status.updated_at_ms - a.status.updated_at_ms)
  const relatedArtifacts = artifacts.filter((artifact) => artifact.workspace?.workspace_id === workspace.workspace_id)
    .sort((a, b) => b.updated_at_ms - a.updated_at_ms).slice(0, 3)
  const unfinished = sessions.filter((entry) => entry.status.run_state !== 'finished').length
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
        <dt className="text-mute">{t('workspaceRuntime')}</dt><dd className="break-all">{workspace.location.runtime_id} · {workspace.runtime_host}</dd>
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
        <button className="tap border border-line text-xs" onClick={() => setEditing(true)} disabled={busy}>{t('workspaceRelocate')}</button>
      </div>
      {editing && <RelocateDialog workspace={workspace} onClose={() => setEditing(false)} refresh={refresh} />}
    </li>
  )
}

export function WorkspaceList({ entries = [] }: { entries?: RegistryEntry[] }) {
  const workspaces = useQuery(['home.workspaces'], () => dataModel.workspaces())
  const artifacts = useQuery(['artifacts.list'], () => dataModel.artifacts())
  const records = [...(workspaces.data ?? [])].sort((a, b) => b.updated_at_ms - a.updated_at_ms)
  return (
    <section className="card min-w-0 p-4" data-testid="known-workspaces">
      <h2 className="mb-3 text-base font-semibold">{t('knownWorkspaces')}
        {workspaces.data && <span className="ml-1.5 text-sm font-normal text-mute">{records.length}</span>}
      </h2>
      <ErrorBox error={workspaces.error ?? artifacts.error} />
      {records.length > 0 ? <ul className="divide-y divide-line">
        {records.map((workspace) => <WorkspaceCard key={workspace.workspace_id} workspace={workspace} entries={entries}
          artifacts={artifacts.data ?? []} refresh={() => workspaces.mutate()} />)}
      </ul> : !workspaces.error && <p className="py-2 text-sm text-mute">{t(workspaces.isLoading ? 'loading' : 'noWorkspaces')}</p>}
    </section>
  )
}
