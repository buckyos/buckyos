/* Workspace list: new (blank or from a template, through the same "New" dialog as inside a workspace),
 * open, import (explicit semantics), export, fork, delete. */

import { useCallback, useState } from 'react'
import { Plus, RefreshCw } from 'lucide-react'
import { unwrap, type AiwsClient } from '../api/client'
import { describeError } from '../api/session'
import type { WorkspaceSummary } from '../api/types'
import { preparedIndex } from '../offline/holder'
import { useLoad } from '../state/hooks'
import { useAiwsText } from '../useAiwsText'
import { ExportForm, ImportForm, NewDialog, type NewTab } from './shell/dialogs'

interface Props { client: AiwsClient; onOpen: (workspaceId: string) => void; opening: string | null }

type CardAction = 'fork' | 'delete'
const ACTION_BUSY: Record<CardAction, string> = { fork: 'aiworkspace.core.list.forking', delete: 'aiworkspace.core.list.deleting' }
const ACTION_FAILED: Record<CardAction, string> = { fork: 'aiworkspace.core.list.forkFailed', delete: 'aiworkspace.core.list.deleteFailed' }

export function WorkspaceList({ client, onOpen, opening }: Props) {
  const { t } = useAiwsText()
  const load = useCallback(async () => unwrap(await client.wsList()).workspaces, [client])
  const list = useLoad<WorkspaceSummary[]>(load)
  const [busy, setBusy] = useState<CardAction | null>(null)
  const [message, setMessage] = useState<{ kind: 'info' | 'error'; text: string } | null>(null)
  const [creating, setCreating] = useState<NewTab | null>(null)

  const act = async (action: CardAction, work: () => Promise<string | void>) => {
    setBusy(action)
    setMessage(null)
    try {
      const text = await work()
      if (text) setMessage({ kind: 'info', text })
      list.reload()
    } catch (error) {
      setMessage({ kind: 'error', text: t(ACTION_FAILED[action], { error: describeError(error) }) })
    } finally {
      setBusy(null)
    }
  }

  return (
    <div className="aiws-list" data-testid="aiws-list">
      <header className="aiws-topbar">
        <b className="aiws-title">{t('aiworkspace.core.list.title')}</b>
        <span className="aiws-muted" data-testid="aiws-transport">{client.transport.mode === 'dev-override' ? t('aiworkspace.core.list.transportDev', { principal: client.transport.principalHint ?? '' }) : t('aiworkspace.core.list.transportZone')}</span>
        <span className="aiws-grow" />
        <button type="button" className="is-primary" data-testid="aiws-new" onClick={() => setCreating('workspace')}><Plus size={16} /> {t('aiworkspace.core.list.new')}</button>
        <button type="button" data-testid="aiws-new-template" onClick={() => setCreating('template')}>{t('aiworkspace.core.list.newFromTemplate')}</button>
        <button type="button" aria-label={t('aiworkspace.common.refresh')} title={t('aiworkspace.common.refresh')} onClick={list.reload}><RefreshCw size={16} /></button>
      </header>
      {message && <div className={message.kind === 'error' ? 'aiws-error' : 'aiws-warning'} role={message.kind === 'error' ? 'alert' : 'status'} data-testid="aiws-list-message">{message.text}</div>}
      {list.error && <div className="aiws-error" role="alert" data-testid="aiws-list-error">{t('aiworkspace.core.list.loadFailed', { error: list.error })}</div>}
      {list.error && !list.data && <PreparedList onOpen={onOpen} opening={opening} principal={client.transport.principalHint} />}
      {busy && <span className="aiws-muted" data-testid="aiws-list-busy">{t(ACTION_BUSY[busy])}</span>}
      <div className="aiws-cards">
        {list.data?.length === 0 && <div className="aiws-muted">{t('aiworkspace.core.list.empty')}</div>}
        {list.data?.map((workspace) => (
          <WorkspaceCard key={workspace.workspace_id} client={client} workspace={workspace} busy={busy !== null} opening={opening === workspace.workspace_id} onOpen={() => onOpen(workspace.workspace_id)} act={act} />
        ))}
      </div>
      <ImportForm client={client} onDone={(text) => { setMessage({ kind: 'info', text }); list.reload() }} />
      {creating && <NewDialog client={client} store={null} initialTab={creating} onClose={() => setCreating(null)} onOpenWorkspace={onOpen} />}
    </div>
  )
}

function WorkspaceCard({ client, workspace, busy, opening, onOpen, act }: {
  client: AiwsClient
  workspace: WorkspaceSummary
  busy: boolean
  opening: boolean
  onOpen: () => void
  act: (action: CardAction, work: () => Promise<string | void>) => Promise<void>
}) {
  const { t } = useAiwsText()
  const [confirmDelete, setConfirmDelete] = useState(false)
  const ws = { workspace_id: workspace.workspace_id }
  const can = (capability: string) => workspace.capabilities.includes(capability as never)
  return (
    <div className="aiws-card" data-testid="aiws-workspace-card" data-workspace-id={workspace.workspace_id}>
      <div className="aiws-card-title">{workspace.title} {preparedIndex()[workspace.workspace_id] && <span className="aiws-chip" data-testid="aiws-prepared-chip" title={t('aiworkspace.core.list.preparedChipTitle')}>{t('aiworkspace.core.list.preparedChip')}</span>}</div>
      <div className="aiws-muted">{t('aiworkspace.core.list.headSeq', { id: workspace.workspace_id, seq: workspace.head_seq })}</div>
      <div className="aiws-inline-form">
        <button type="button" className="is-primary" data-testid="aiws-open" disabled={opening} onClick={onOpen}>{opening ? t('aiworkspace.core.list.opening') : t('aiworkspace.core.list.open')}</button>
        {can('manage') && <button type="button" data-testid="aiws-fork" disabled={busy} onClick={() => { void act('fork', async () => { const forked = unwrap(await client.wsFork(ws)); return t('aiworkspace.core.list.forked', { id: forked.workspace_id }) }) }}>{t('aiworkspace.core.list.fork')}</button>}
        {can('manage') && (confirmDelete ? (
          <>
            <button type="button" className="is-danger" data-testid="aiws-delete-confirm" disabled={busy} onClick={() => { void act('delete', async () => { unwrap(await client.wsDelete(ws)); setConfirmDelete(false) }) }}>{t('aiworkspace.core.list.confirmDelete')}</button>
            <button type="button" onClick={() => setConfirmDelete(false)}>{t('aiworkspace.common.cancel')}</button>
          </>
        ) : <button type="button" data-testid="aiws-delete" disabled={busy} onClick={() => setConfirmDelete(true)}>{t('aiworkspace.core.list.delete')}</button>)}
      </div>
      {can('export') && <ExportForm client={client} workspace={workspace} />}
    </div>
  )
}

/** The backend cannot be reached: the Workspaces prepared for offline on this device can still be opened. */
function PreparedList({ onOpen, opening, principal }: { onOpen: (workspaceId: string) => void; opening: string | null; principal: string | null }) {
  const { t } = useAiwsText()
  const prepared = Object.values(preparedIndex()).filter((hint) => !principal || hint.principal === principal)
  return (
    <div className="aiws-cards" data-testid="aiws-prepared-list">
      <div className="aiws-warning" role="status">
        {t('aiworkspace.core.list.preparedStatus', {
          network: navigator.onLine === false ? t('aiworkspace.core.list.browserOffline') : t('aiworkspace.core.list.backendUnreachable'),
          prepared: prepared.length > 0 ? t('aiworkspace.core.list.preparedHint') : t('aiworkspace.core.list.noneOffline'),
        })}
      </div>
      {prepared.map((hint) => (
        <div key={hint.workspace_id} className="aiws-card" data-testid="aiws-workspace-card" data-workspace-id={hint.workspace_id} data-offline="true">
          <div className="aiws-card-title">{hint.title} <span className="aiws-chip">{t('aiworkspace.core.list.offlineCopy')}</span></div>
          <div className="aiws-muted">{t('aiworkspace.core.list.preparedAt', { id: hint.workspace_id, at: hint.prepared_at })}</div>
          <div className="aiws-inline-form">
            <button type="button" data-testid="aiws-open" disabled={opening === hint.workspace_id} onClick={() => onOpen(hint.workspace_id)}>{opening === hint.workspace_id ? t('aiworkspace.core.list.opening') : t('aiworkspace.core.list.openOffline')}</button>
          </div>
        </div>
      ))}
    </div>
  )
}
