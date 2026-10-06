/* Workspace list: create, sample, open, import (explicit semantics), export, fork, delete. */

import { useCallback, useState } from 'react'
import { unwrap, type AiwsClient } from '../api/client'
import { createSampleWorkspace } from '../api/sample'
import { createDemoWorkspace } from '../api/demos'
import { describeError } from '../api/session'
import type { ExportResult, WorkspaceSummary } from '../api/types'
import { preparedIndex } from '../offline/holder'
import { useLoad } from '../state/hooks'

interface Props { client: AiwsClient; onOpen: (workspaceId: string) => void; opening: string | null }

function saveBlob(blob: Blob, fileName: string) {
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = fileName
  document.body.appendChild(link)
  link.click()
  link.remove()
  window.setTimeout(() => URL.revokeObjectURL(url), 10_000)
}

export function WorkspaceList({ client, onOpen, opening }: Props) {
  const load = useCallback(async () => unwrap(await client.wsList()).workspaces, [client])
  const list = useLoad<WorkspaceSummary[]>(load)
  const [title, setTitle] = useState('')
  const [busy, setBusy] = useState<string | null>(null)
  const [message, setMessage] = useState<{ kind: 'info' | 'error'; text: string } | null>(null)
  const [semantics, setSemantics] = useState<'' | 'new' | 'restore'>('')
  const [replace, setReplace] = useState(false)
  const [file, setFile] = useState<File | null>(null)

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

  const importPackage = () => act('导入', async () => {
    if (!file || semantics === '') return
    const begin = unwrap(await client.wsBeginImport())
    await client.transport.upload(begin.upload_id, file)
    const result = unwrap(await client.wsImport(begin.upload_id, semantics, semantics === 'restore' && replace))
    setFile(null)
    return semantics === 'new' ? `已导入为新的工作区 ${result.workspace_id}。` : `已恢复工作区 ${result.workspace_id}（历史代次已更换，旧窗口需重新打开）。`
  })

  return (
    <div className="aiws-list" data-testid="aiws-list">
      <header className="aiws-topbar">
        <b className="aiws-title">AI 工作区</b>
        <span className="aiws-muted" data-testid="aiws-transport">{client.transport.mode === 'dev-override' ? `开发直连（${client.transport.principalHint ?? ''}）` : 'Zone 服务'}</span>
        <span className="aiws-grow" />
        <button type="button" onClick={list.reload}>刷新</button>
      </header>
      {message && <div className={message.kind === 'error' ? 'aiws-error' : 'aiws-warning'} role={message.kind === 'error' ? 'alert' : 'status'} data-testid="aiws-list-message">{message.text}</div>}
      {list.error && <div className="aiws-error" role="alert" data-testid="aiws-list-error">无法读取工作区列表：{list.error}</div>}
      {list.error && !list.data && <PreparedList onOpen={onOpen} opening={opening} principal={client.transport.principalHint} />}
      <form className="aiws-inline-form" onSubmit={(event) => {
        event.preventDefault()
        const name = title.trim() || '未命名工作区'
        void act('创建', async () => { unwrap(await client.wsCreate(name)); setTitle('') })
      }}>
        <input aria-label="工作区标题" placeholder="新工作区标题" value={title} onChange={(event) => setTitle(event.target.value)} />
        <button type="submit" data-testid="aiws-create" disabled={busy !== null}>创建空白工作区</button>
        <button type="button" data-testid="aiws-create-sample" disabled={busy !== null} onClick={() => { void act('创建样例', async () => { await createSampleWorkspace(client, title.trim() || '项目工作区（样例）'); setTitle('') }) }}>创建样例</button>
        <button type="button" data-testid="aiws-create-demo-quarterly" disabled={busy !== null} title="季度经营分析：销售表、两张画布、Mock 许愿格与声明式指标卡" onClick={() => { void act('创建 demo', async () => { await createDemoWorkspace(client, 'quarterly', title.trim() || '季度经营分析（demo）'); setTitle('') }) }}>季度经营分析 demo</button>
        <button type="button" data-testid="aiws-create-demo-film" disabled={busy !== null} title="AI 短片工作流：剧本/角色表/风格 → 三个串联的 Mock 许愿格" onClick={() => { void act('创建 demo', async () => { await createDemoWorkspace(client, 'film', title.trim() || 'AI 短片工作流（demo）'); setTitle('') }) }}>AI 短片工作流 demo</button>
        {busy && <span className="aiws-muted" data-testid="aiws-list-busy">{busy}中…</span>}
      </form>
      <div className="aiws-cards">
        {list.data?.length === 0 && <div className="aiws-muted">还没有工作区。创建一个空白工作区，或创建样例看看各类对象。</div>}
        {list.data?.map((workspace) => (
          <WorkspaceCard key={workspace.workspace_id} client={client} workspace={workspace} busy={busy !== null} opening={opening === workspace.workspace_id} onOpen={() => onOpen(workspace.workspace_id)} act={act} />
        ))}
      </div>
      <form className="aiws-import" data-testid="aiws-import" onSubmit={(event) => { event.preventDefault(); void importPackage() }}>
        <div className="aiws-panel-title">导入工作区包</div>
        <input type="file" aria-label="工作区包" accept=".zip,application/zip" data-testid="aiws-import-file" onChange={(event) => setFile(event.target.files?.[0] ?? null)} />
        <fieldset>
          <legend>导入语义（必须选择）</legend>
          <label><input type="radio" name="aiws-semantics" value="new" checked={semantics === 'new'} onChange={() => setSemantics('new')} /> 作为新的工作区（新 ID、新协作历史）</label>
          <label><input type="radio" name="aiws-semantics" value="restore" checked={semantics === 'restore'} onChange={() => setSemantics('restore')} /> 恢复为包内的同一工作区（个人恢复备份）</label>
          {semantics === 'restore' && <label><input type="checkbox" checked={replace} onChange={(event) => setReplace(event.target.checked)} /> 若该工作区已存在则替换（现有内容移入回收目录）</label>}
        </fieldset>
        <button type="submit" data-testid="aiws-import-submit" disabled={!file || semantics === '' || busy !== null}>导入</button>
      </form>
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
  const [mode, setMode] = useState<'share' | 'personal_backup'>('share')
  const [selfContained, setSelfContained] = useState(true)
  const [confirmDelete, setConfirmDelete] = useState(false)
  const [exported, setExported] = useState<ExportResult | null>(null)
  const ws = { workspace_id: workspace.workspace_id }
  const can = (capability: string) => workspace.capabilities.includes(capability as never)
  return (
    <div className="aiws-card" data-testid="aiws-workspace-card" data-workspace-id={workspace.workspace_id}>
      <div className="aiws-card-title">{workspace.title} {preparedIndex()[workspace.workspace_id] && <span className="aiws-chip" data-testid="aiws-prepared-chip" title="本设备上有这个工作区的离线副本">已准备离线</span>}</div>
      <div className="aiws-muted">{workspace.workspace_id} · 第 {workspace.head_seq} 次提交</div>
      <div className="aiws-inline-form">
        <button type="button" data-testid="aiws-open" disabled={opening} onClick={onOpen}>{opening ? '打开中…' : '打开'}</button>
        {can('export') && (
          <>
            <select aria-label="导出方式" value={mode} onChange={(event) => setMode(event.target.value === 'personal_backup' ? 'personal_backup' : 'share')}>
              <option value="share">分享导出（不含协作历史）</option>
              <option value="personal_backup">个人恢复备份（含协作历史）</option>
            </select>
            <label><input type="checkbox" checked={selfContained} onChange={(event) => setSelfContained(event.target.checked)} /> 自包含</label>
            <button type="button" data-testid="aiws-export" disabled={busy} onClick={() => {
              void act('导出', async () => {
                const result = unwrap(await client.export(ws, mode, selfContained))
                const blob = await client.transport.download(`export/${workspace.workspace_id}/${result.export_id}`)
                saveBlob(blob, `${workspace.title}-${mode === 'share' ? 'share' : 'backup'}.zip`)
                setExported(result)
                return `已导出「${workspace.title}」（${blob.size} 字节）。`
              })
            }}>导出并下载</button>
          </>
        )}
        {can('manage') && <button type="button" data-testid="aiws-fork" disabled={busy} onClick={() => { void act('Fork', async () => { const forked = unwrap(await client.wsFork(ws)); return `已 Fork 为新的工作区 ${forked.workspace_id}。` }) }}>Fork</button>}
        {can('manage') && (confirmDelete ? (
          <>
            <button type="button" data-testid="aiws-delete-confirm" disabled={busy} onClick={() => { void act('删除', async () => { unwrap(await client.wsDelete(ws)); setConfirmDelete(false) }) }}>确认删除（移入回收目录）</button>
            <button type="button" onClick={() => setConfirmDelete(false)}>取消</button>
          </>
        ) : <button type="button" data-testid="aiws-delete" disabled={busy} onClick={() => setConfirmDelete(true)}>删除</button>)}
      </div>
      {exported && (
        <div className="aiws-muted" data-testid="aiws-export-manifest">
          上次导出：{exported.manifest.export_mode === 'share' ? '分享' : '个人恢复备份'} · 内容根 {exported.manifest.content_root}
          {exported.manifest.missing.length > 0 && ` · 缺失 ${exported.manifest.missing.length} 项`}
          {exported.manifest.excluded_entities > 0 && ` · 未包含 ${exported.manifest.excluded_entities} 个无权读取的对象`}
        </div>
      )}
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
