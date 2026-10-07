/* Workspace list: new (blank or from a template, through the same "New" dialog as inside a workspace),
 * open, import (explicit semantics), export, fork, delete. */

import { useCallback, useState } from 'react'
import { Plus, RefreshCw } from 'lucide-react'
import { unwrap, type AiwsClient } from '../api/client'
import { describeError } from '../api/session'
import type { WorkspaceSummary } from '../api/types'
import { preparedIndex } from '../offline/holder'
import { useLoad } from '../state/hooks'
import { ExportForm, ImportForm, NewDialog, type NewTab } from './shell/dialogs'

interface Props { client: AiwsClient; onOpen: (workspaceId: string) => void; opening: string | null }

export function WorkspaceList({ client, onOpen, opening }: Props) {
  const load = useCallback(async () => unwrap(await client.wsList()).workspaces, [client])
  const list = useLoad<WorkspaceSummary[]>(load)
  const [busy, setBusy] = useState<string | null>(null)
  const [message, setMessage] = useState<{ kind: 'info' | 'error'; text: string } | null>(null)
  const [creating, setCreating] = useState<NewTab | null>(null)

  const act = async (label: string, work: () => Promise<string | void>) => {
    setBusy(label)
    setMessage(null)
    try {
      const text = await work()
      if (text) setMessage({ kind: 'info', text })
      list.reload()
    } catch (error) {
      setMessage({ kind: 'error', text: `${label}失败：${describeError(error)}` })
    } finally {
      setBusy(null)
    }
  }

  return (
    <div className="aiws-list" data-testid="aiws-list">
      <header className="aiws-topbar">
        <b className="aiws-title">AI 工作区</b>
        <span className="aiws-muted" data-testid="aiws-transport">{client.transport.mode === 'dev-override' ? `开发直连（${client.transport.principalHint ?? ''}）` : 'Zone 服务'}</span>
        <span className="aiws-grow" />
        <button type="button" className="is-primary" data-testid="aiws-new" onClick={() => setCreating('workspace')}><Plus size={16} /> 新建…</button>
        <button type="button" data-testid="aiws-new-template" onClick={() => setCreating('template')}>从模板新建…</button>
        <button type="button" aria-label="刷新" title="刷新" onClick={list.reload}><RefreshCw size={16} /></button>
      </header>
      {message && <div className={message.kind === 'error' ? 'aiws-error' : 'aiws-warning'} role={message.kind === 'error' ? 'alert' : 'status'} data-testid="aiws-list-message">{message.text}</div>}
      {list.error && <div className="aiws-error" role="alert" data-testid="aiws-list-error">无法读取工作区列表：{list.error}</div>}
      {list.error && !list.data && <PreparedList onOpen={onOpen} opening={opening} principal={client.transport.principalHint} />}
      {busy && <span className="aiws-muted" data-testid="aiws-list-busy">{busy}中…</span>}
      <div className="aiws-cards">
        {list.data?.length === 0 && <div className="aiws-muted">还没有工作区。新建一个空白工作区，或从模板创建，看看各类对象。</div>}
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
  act: (label: string, work: () => Promise<string | void>) => Promise<void>
}) {
  const [confirmDelete, setConfirmDelete] = useState(false)
  const ws = { workspace_id: workspace.workspace_id }
  const can = (capability: string) => workspace.capabilities.includes(capability as never)
  return (
    <div className="aiws-card" data-testid="aiws-workspace-card" data-workspace-id={workspace.workspace_id}>
      <div className="aiws-card-title">{workspace.title} {preparedIndex()[workspace.workspace_id] && <span className="aiws-chip" data-testid="aiws-prepared-chip" title="本设备上有这个工作区的离线副本">已准备离线</span>}</div>
      <div className="aiws-muted">{workspace.workspace_id} · 第 {workspace.head_seq} 次提交</div>
      <div className="aiws-inline-form">
        <button type="button" className="is-primary" data-testid="aiws-open" disabled={opening} onClick={onOpen}>{opening ? '打开中…' : '打开'}</button>
        {can('manage') && <button type="button" data-testid="aiws-fork" disabled={busy} onClick={() => { void act('Fork', async () => { const forked = unwrap(await client.wsFork(ws)); return `已 Fork 为新的工作区 ${forked.workspace_id}。` }) }}>Fork</button>}
        {can('manage') && (confirmDelete ? (
          <>
            <button type="button" className="is-danger" data-testid="aiws-delete-confirm" disabled={busy} onClick={() => { void act('删除', async () => { unwrap(await client.wsDelete(ws)); setConfirmDelete(false) }) }}>确认删除工作区（移入回收目录）</button>
            <button type="button" onClick={() => setConfirmDelete(false)}>取消</button>
          </>
        ) : <button type="button" data-testid="aiws-delete" disabled={busy} onClick={() => setConfirmDelete(true)}>删除工作区</button>)}
      </div>
      {can('export') && <ExportForm client={client} workspace={workspace} />}
    </div>
  )
}

/** The backend cannot be reached: the Workspaces prepared for offline on this device can still be opened. */
function PreparedList({ onOpen, opening, principal }: { onOpen: (workspaceId: string) => void; opening: string | null; principal: string | null }) {
  const prepared = Object.values(preparedIndex()).filter((hint) => !principal || hint.principal === principal)
  return (
    <div className="aiws-cards" data-testid="aiws-prepared-list">
      <div className="aiws-warning" role="status">
        {navigator.onLine === false ? '浏览器处于离线状态。' : '浏览器网络在线，但连接不到 BuckyOS 后台。'}
        {prepared.length > 0 ? '以下工作区已在本设备准备离线，可以打开并编辑；修改保存在本设备，恢复连接后提交。' : '本设备上没有准备过离线的工作区。'}
      </div>
      {prepared.map((hint) => (
        <div key={hint.workspace_id} className="aiws-card" data-testid="aiws-workspace-card" data-workspace-id={hint.workspace_id} data-offline="true">
          <div className="aiws-card-title">{hint.title} <span className="aiws-chip">离线副本</span></div>
          <div className="aiws-muted">{hint.workspace_id} · 准备于 {hint.prepared_at}</div>
          <div className="aiws-inline-form">
            <button type="button" data-testid="aiws-open" disabled={opening === hint.workspace_id} onClick={() => onOpen(hint.workspace_id)}>{opening === hint.workspace_id ? '打开中…' : '打开（离线副本）'}</button>
          </div>
        </div>
      ))}
    </div>
  )
}
