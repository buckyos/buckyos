/* Shell dialogs (UI improvement §6.2, §10, §11): the unified "New" dialog (canvas / workspace /
 * template), export, import, keyboard help, the developer Mock panel and the leave check. The New
 * dialog also serves the workspace list, where only the workspace and template tabs exist. */

import { useState, type ReactNode } from 'react'
import { unwrap, type AiwsClient } from '../../api/client'
import { createDemoWorkspace, TemplateFailure } from '../../api/demos'
import { createSampleWorkspace } from '../../api/sample'
import { describeError } from '../../api/session'
import type { ExportResult, WorkspaceSummary } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import { SURFACE_ICONS } from '../canvas/icons'
import { accepted } from '../canvas/surfaceManage'
import { createSurfaceOps, surfacesOf } from '../canvas/surfaceOps'
import { MockRunPanel } from './panels'
import { useOverlayMounted } from './popover'

export function Modal({ label, testId, onClose, children, className = '', modal = true }: { label: string; testId?: string; onClose: () => void; children: ReactNode; className?: string; modal?: boolean }) {
  useOverlayMounted(modal)
  const box = (
    <div className={`aiws-dialog ${className}`} role="dialog" aria-modal={modal} aria-label={label} data-testid={testId}
      onKeyDown={(event) => { if (event.key === 'Escape') { event.stopPropagation(); onClose() } }}>
      {children}
    </div>
  )
  if (!modal) return box
  return <div className="aiws-modal-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose() }}>{box}</div>
}

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

// ---- New

export type NewTab = 'canvas' | 'workspace' | 'template'

const TEMPLATES = [
  { id: 'sample' as const, title: '项目工作区样例', description: '任务表、会议纪要（富文本）、项目记录、图片附件和批注：看看各类对象如何协作。', preview: '流式页 1 张 · 表格 · 富文本 · 记录 · 图片 · 批注', mock: false },
  { id: 'quarterly' as const, title: '季度经营分析', description: '销售表、两张自由画布、模拟许愿格与声明式指标卡。', preview: '自由画布 2 张 · 销售表 · 图表 · 许愿格（模拟）· 声明式扩展', mock: true },
  { id: 'film' as const, title: 'AI 短片工作流', description: '剧本、角色表、风格设定，经三个串联的模拟许愿格生成分镜与预览。', preview: '自由画布 1 张 · 剧本 · 角色表 · 3 个串联许愿格（模拟）', mock: true },
]

export function NewDialog({ client, store, initialTab, onClose, onCreatedSurface, onOpenWorkspace }: {
  client: AiwsClient
  /** The open workspace (the canvas tab needs it); null in the workspace list. */
  store: WorkspaceStore | null
  initialTab: NewTab
  onClose: () => void
  onCreatedSurface?: (surfaceId: string) => void
  onOpenWorkspace: (workspaceId: string) => void
}) {
  const canCanvas = Boolean(store?.session.info().capabilities.includes('structure'))
  const [tab, setTab] = useState<NewTab>(initialTab === 'canvas' && !canCanvas ? 'workspace' : initialTab)
  const tabs: NewTab[] = store ? ['canvas', 'workspace', 'template'] : ['workspace', 'template']
  const label: Record<NewTab, string> = { canvas: '画布', workspace: '工作区', template: '模板' }
  return (
    <Modal label="新建" testId="aiws-new-dialog" onClose={onClose} className="aiws-new-dialog">
      <div className="aiws-dialog-head"><b>新建</b><span className="aiws-grow" /><button type="button" className="aiws-link" onClick={onClose}>关闭</button></div>
      <div className="aiws-tabs" role="tablist" aria-label="新建什么">
        {tabs.map((t) => <button key={t} type="button" role="tab" aria-selected={tab === t} data-testid={`aiws-new-tab-${t}`} disabled={t === 'canvas' && !canCanvas} title={t === 'canvas' && !canCanvas ? '没有新建画布的权限' : undefined} onClick={() => setTab(t)}>{label[t]}</button>)}
      </div>
      {tab === 'canvas' && store && <NewCanvas store={store} onDone={(id) => { onClose(); onCreatedSurface?.(id) }} />}
      {tab === 'workspace' && <NewWorkspace client={client} onOpen={(id) => { onClose(); onOpenWorkspace(id) }} />}
      {tab === 'template' && <NewFromTemplate client={client} onOpen={(id) => { onClose(); onOpenWorkspace(id) }} />}
    </Modal>
  )
}

function NewCanvas({ store, onDone }: { store: WorkspaceStore; onDone: (surfaceId: string) => void }) {
  const [title, setTitle] = useState('')
  const [layout, setLayout] = useState<'free' | 'flow'>('free')
  const [icon, setIcon] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const create = async () => {
    const name = title.trim() || `画布 ${surfacesOf(store).length + 1}`
    setBusy(true)
    setError(null)
    const { ops, surfaceId } = createSurfaceOps(store, name, layout, icon)
    const outcome = await store.submit({ editId: 'surface:new', label: `新建画布 ${name}`, operations: ops })
    setBusy(false)
    if (accepted(outcome)) onDone(surfaceId)
    else setError(`没有创建：${'code' in outcome ? outcome.code : outcome.status}`)
  }
  return (
    <form className="aiws-form" onSubmit={(event) => { event.preventDefault(); void create() }}>
      <label className="aiws-field">名称<input autoFocus aria-label="新画布标题" placeholder={`画布 ${surfacesOf(store).length + 1}`} value={title} maxLength={256} onChange={(event) => setTitle(event.target.value)} /></label>
      <fieldset className="aiws-field">
        <legend>布局</legend>
        <label><input type="radio" name="aiws-new-layout" checked={layout === 'free'} onChange={() => setLayout('free')} /> 自由画布（任意摆放、缩放）</label>
        <label><input type="radio" name="aiws-new-layout" checked={layout === 'flow'} onChange={() => setLayout('flow')} data-testid="aiws-new-flow" /> 流式页（按顺序排列）</label>
      </fieldset>
      <fieldset className="aiws-field">
        <legend>图标</legend>
        <div className="aiws-icon-grid" role="listbox" aria-label="预设图标">
          {SURFACE_ICONS.map(({ id, label, Icon }) => <button key={id} type="button" role="option" aria-selected={icon === id} aria-label={label} title={label} onClick={() => setIcon(icon === id ? null : id)}><Icon size={18} /></button>)}
        </div>
      </fieldset>
      <div className="aiws-muted">画布和它的内容区在同一次提交中创建，完成后切换到新画布。</div>
      {error && <div className="aiws-error" role="alert">{error}</div>}
      <div className="aiws-dialog-actions"><button type="submit" className="is-primary" data-testid="aiws-surface-create" disabled={busy}>创建画布</button></div>
    </form>
  )
}

function NewWorkspace({ client, onOpen }: { client: AiwsClient; onOpen: (workspaceId: string) => void }) {
  const [title, setTitle] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const create = async () => {
    setBusy(true)
    setError(null)
    try {
      const created = unwrap(await client.wsCreate(title.trim() || '未命名工作区'))
      onOpen(created.workspace_id)
    } catch (failure) {
      setError(`创建失败：${describeError(failure)}`)
      setBusy(false)
    }
  }
  return (
    <form className="aiws-form" onSubmit={(event) => { event.preventDefault(); void create() }}>
      <label className="aiws-field">名称<input autoFocus aria-label="工作区标题" placeholder="未命名工作区" value={title} onChange={(event) => setTitle(event.target.value)} /></label>
      <div className="aiws-muted">创建空白工作区并打开；打开后可新建第一张自由画布或流式页。</div>
      {error && <div className="aiws-error" role="alert" data-testid="aiws-new-error">{error}</div>}
      <div className="aiws-dialog-actions"><button type="submit" className="is-primary" data-testid="aiws-create" disabled={busy}>{busy ? '创建中…' : '创建并打开'}</button></div>
    </form>
  )
}

function NewFromTemplate({ client, onOpen }: { client: AiwsClient; onOpen: (workspaceId: string) => void }) {
  const [chosen, setChosen] = useState<(typeof TEMPLATES)[number]['id']>('sample')
  const [title, setTitle] = useState('')
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<{ text: string; workspace: WorkspaceSummary | null } | null>(null)
  const template = TEMPLATES.find((t) => t.id === chosen) ?? TEMPLATES[0]
  const create = async () => {
    setBusy(true)
    setFailure(null)
    const name = title.trim() || template.title
    try {
      const created = template.id === 'sample' ? await createSampleWorkspace(client, name) : await createDemoWorkspace(client, template.id, name)
      onOpen(created.workspace_id)
    } catch (error) {
      setFailure(error instanceof TemplateFailure ? { text: `模板没有完整创建（${error.message}）`, workspace: error.workspace } : { text: `创建失败：${describeError(error)}`, workspace: null })
      setBusy(false)
    }
  }
  const removePartial = async (workspace: WorkspaceSummary) => {
    try { unwrap(await client.wsDelete({ workspace_id: workspace.workspace_id })); setFailure({ text: `已删除不完整的工作区「${workspace.title}」（移入回收目录）。`, workspace: null }) } catch (error) { setFailure({ text: `删除失败：${describeError(error)}`, workspace }) }
  }
  return (
    <form className="aiws-form" onSubmit={(event) => { event.preventDefault(); void create() }}>
      <div className="aiws-template-list" role="radiogroup" aria-label="模板">
        {TEMPLATES.map((t) => (
          <label key={t.id} className={`aiws-template${chosen === t.id ? ' is-active' : ''}`} data-testid={`aiws-template-${t.id}`}>
            <input type="radio" name="aiws-template" checked={chosen === t.id} onChange={() => setChosen(t.id)} />
            <span className="aiws-template-text">
              <span className="aiws-template-title"><b>{t.title}</b>{t.mock && <span className="aiws-chip aiws-chip-derived" title="其中的许愿格使用模拟执行器，结果是预先准备的">含模拟内容</span>}</span>
              <span>{t.description}</span>
              <span className="aiws-muted">{t.preview}</span>
            </span>
          </label>
        ))}
      </div>
      <label className="aiws-field">新工作区名称<input aria-label="工作区标题" placeholder={template.title} value={title} onChange={(event) => setTitle(event.target.value)} /></label>
      <div className="aiws-muted">从模板创建<b>新的工作区</b>并打开它；不会改动当前工作区。打开模板不会自动运行许愿格。</div>
      {failure && (
        <div className="aiws-error" role="alert" data-testid="aiws-template-failure">
          {failure.text}
          {failure.workspace && (
            <>
              <button type="button" onClick={() => onOpen(failure.workspace?.workspace_id ?? '')}>打开已创建的部分</button>
              <button type="button" onClick={() => { if (failure.workspace) void removePartial(failure.workspace) }}>删除它</button>
            </>
          )}
        </div>
      )}
      <div className="aiws-dialog-actions"><button type="submit" className="is-primary" data-testid="aiws-create-template" disabled={busy}>{busy ? '正在创建…' : '从模板创建工作区'}</button></div>
    </form>
  )
}

// ---- export / import

export function ExportForm({ client, workspace, onDone }: { client: AiwsClient; workspace: { workspace_id: string; title: string }; onDone?: (message: string) => void }) {
  const [mode, setMode] = useState<'share' | 'personal_backup'>('share')
  const [selfContained, setSelfContained] = useState(true)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<{ kind: 'info' | 'error'; text: string } | null>(null)
  const [exported, setExported] = useState<ExportResult | null>(null)
  const run = async () => {
    setBusy(true)
    setMessage(null)
    try {
      const result = unwrap(await client.export({ workspace_id: workspace.workspace_id }, mode, selfContained))
      const blob = await client.transport.download(`export/${workspace.workspace_id}/${result.export_id}`)
      saveBlob(blob, `${workspace.title}-${mode === 'share' ? 'share' : 'backup'}.zip`)
      setExported(result)
      const text = `已导出「${workspace.title}」（${blob.size} 字节）。`
      setMessage({ kind: 'info', text })
      onDone?.(text)
    } catch (error) {
      setMessage({ kind: 'error', text: `导出失败：${describeError(error)}` })
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="aiws-inline-form">
      <select aria-label="导出方式" value={mode} onChange={(event) => setMode(event.target.value === 'personal_backup' ? 'personal_backup' : 'share')}>
        <option value="share">分享包（不含协作历史）</option>
        <option value="personal_backup">个人恢复备份（含协作历史）</option>
      </select>
      <label><input type="checkbox" checked={selfContained} onChange={(event) => setSelfContained(event.target.checked)} /> 自包含</label>
      <button type="button" data-testid="aiws-export" disabled={busy} onClick={() => { void run() }}>{busy ? '导出中…' : '导出并下载'}</button>
      {message && <div className={message.kind === 'error' ? 'aiws-error' : 'aiws-muted'} role={message.kind === 'error' ? 'alert' : 'status'} data-testid="aiws-export-message">{message.text}</div>}
      {exported && (
        <div className="aiws-muted" data-testid="aiws-export-manifest">
          上次导出：{exported.manifest.export_mode === 'share' ? '分享包' : '个人恢复备份'} · 内容根 {exported.manifest.content_root}
          {exported.manifest.missing.length > 0 && ` · 缺失 ${exported.manifest.missing.length} 项`}
          {exported.manifest.excluded_entities > 0 && ` · 未包含 ${exported.manifest.excluded_entities} 个无权读取的对象`}
        </div>
      )}
    </div>
  )
}

export function ImportForm({ client, onDone }: { client: AiwsClient; onDone: (message: string, workspaceId: string) => void }) {
  const [semantics, setSemantics] = useState<'' | 'new' | 'restore'>('')
  const [replace, setReplace] = useState(false)
  const [file, setFile] = useState<File | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const run = async () => {
    if (!file || semantics === '') return
    setBusy(true)
    setError(null)
    try {
      const begin = unwrap(await client.wsBeginImport())
      await client.transport.upload(begin.upload_id, file)
      const result = unwrap(await client.wsImport(begin.upload_id, semantics, semantics === 'restore' && replace))
      setFile(null)
      onDone(semantics === 'new' ? `已导入为新的工作区 ${result.workspace_id}。` : `已恢复工作区 ${result.workspace_id}（历史代次已更换，旧窗口需重新打开）。`, result.workspace_id)
    } catch (failure) {
      setError(`导入失败：${describeError(failure)}`)
    } finally {
      setBusy(false)
    }
  }
  return (
    <form className="aiws-import" data-testid="aiws-import" onSubmit={(event) => { event.preventDefault(); void run() }}>
      <div className="aiws-panel-title">导入工作区包</div>
      <input type="file" aria-label="工作区包" accept=".zip,application/zip" data-testid="aiws-import-file" onChange={(event) => setFile(event.target.files?.[0] ?? null)} />
      <fieldset>
        <legend>导入语义（必须选择）</legend>
        <label><input type="radio" name="aiws-semantics" value="new" checked={semantics === 'new'} onChange={() => setSemantics('new')} /> 作为新的工作区（新 ID、新协作历史）</label>
        <label><input type="radio" name="aiws-semantics" value="restore" checked={semantics === 'restore'} onChange={() => setSemantics('restore')} /> 恢复为包内的同一工作区（个人恢复备份）</label>
        {semantics === 'restore' && <label><input type="checkbox" checked={replace} onChange={(event) => setReplace(event.target.checked)} /> 若该工作区已存在则替换（现有内容移入回收目录）</label>}
      </fieldset>
      {error && <div className="aiws-error" role="alert" data-testid="aiws-import-error">{error}</div>}
      <button type="submit" data-testid="aiws-import-submit" disabled={!file || semantics === '' || busy}>{busy ? '导入中…' : '导入'}</button>
    </form>
  )
}

export function ExportDialog({ client, workspace, onClose }: { client: AiwsClient; workspace: { workspace_id: string; title: string }; onClose: () => void }) {
  return (
    <Modal label="导出工作区" testId="aiws-export-dialog" onClose={onClose}>
      <div className="aiws-dialog-head"><b>导出「{workspace.title}」</b><span className="aiws-grow" /><button type="button" className="aiws-link" onClick={onClose}>关闭</button></div>
      <div className="aiws-muted">分享包给别人导入成新的工作区，不含协作历史和无权读取的对象；个人恢复备份用于恢复同一个工作区。这与“分享链接”不同：链接只指向线上工作区，不复制内容。</div>
      <ExportForm client={client} workspace={workspace} />
    </Modal>
  )
}

export function ImportDialog({ client, onClose, onOpenWorkspace }: { client: AiwsClient; onClose: () => void; onOpenWorkspace: (workspaceId: string) => void }) {
  const [done, setDone] = useState<{ text: string; workspaceId: string } | null>(null)
  return (
    <Modal label="导入工作区包" testId="aiws-import-dialog" onClose={onClose}>
      <div className="aiws-dialog-head"><b>导入工作区包</b><span className="aiws-grow" /><button type="button" className="aiws-link" onClick={onClose}>关闭</button></div>
      {done ? (
        <>
          <div role="status">{done.text}</div>
          <div className="aiws-dialog-actions"><button type="button" className="is-primary" onClick={() => { onClose(); onOpenWorkspace(done.workspaceId) }}>打开导入的工作区</button><button type="button" onClick={onClose}>留在当前工作区</button></div>
        </>
      ) : <ImportForm client={client} onDone={(text, workspaceId) => setDone({ text, workspaceId })} />}
    </Modal>
  )
}

// ---- help, developer tools, leave check

const SHORTCUTS: [string, string][] = [
  ['Ctrl/⌘ + Z', '撤销（一次一步）'], ['Ctrl/⌘ + Shift + Z，Ctrl/⌘ + Y', '重做'],
  ['Ctrl/⌘ + C / X / V', '复制 / 剪切 / 粘贴画布对象（焦点在文字或表格中时由编辑器处理）'], ['Delete / Backspace', '删除选中的对象'],
  ['Enter', '编辑选中对象的内容'], ['Esc', '逐层退出：菜单、放置、编辑、选择'], ['方向键（Shift 加速）', '移动选中的对象'],
  ['Ctrl/⌘ + G，Ctrl/⌘ + Shift + G', '分组 / 解组'], ['F2', '打开属性'], ['V / H', '选择工具 / 移动视图工具'],
  ['按住空格拖动，鼠标中键拖动', '临时移动视图'], ['Ctrl/⌘ + 滚轮', '以指针为中心缩放'], ['滚轮，Shift + 滚轮', '上下 / 左右移动视图'],
]

export function HelpDialog({ onClose }: { onClose: () => void }) {
  return (
    <Modal label="快捷键与操作说明" testId="aiws-help-dialog" onClose={onClose}>
      <div className="aiws-dialog-head"><b>快捷键与操作说明</b><span className="aiws-grow" /><button type="button" className="aiws-link" autoFocus onClick={onClose}>关闭</button></div>
      <table className="aiws-shortcuts"><tbody>{SHORTCUTS.map(([keys, text]) => <tr key={keys}><th scope="row"><kbd>{keys}</kbd></th><td>{text}</td></tr>)}</tbody></table>
      <div className="aiws-muted">画布快捷键只在画布获得焦点时生效；输入法组合输入期间不响应。缩放与视图导航在右上角的比例菜单中。</div>
    </Modal>
  )
}

export function MockDialog({ onClose }: { onClose: () => void }) {
  return (
    <Modal label="受控加工（开发工具）" testId="aiws-mock-dialog" onClose={onClose} modal={false} className="aiws-floating-dialog">
      <div className="aiws-dialog-head"><b>开发工具</b><span className="aiws-grow" /><button type="button" className="aiws-link" onClick={onClose}>关闭</button></div>
      <MockRunPanel />
    </Modal>
  )
}

export interface LeaveSummary { unsaved: number; attention: number; memoryOnly: number }

export function LeaveDialog({ summary, reason, onStay, onExportAndLeave, onLeave }: { summary: LeaveSummary; reason: string; onStay: () => void; onExportAndLeave: () => void; onLeave: () => void }) {
  return (
    <Modal label="还有没保存的修改" testId="aiws-leave-dialog" onClose={onStay}>
      <b>{reason}前还有没保存好的内容</b>
      <ul>
        {summary.memoryOnly > 0 && <li>{summary.memoryOnly} 项输入只在这个窗口的内存中，离开后会丢失。</li>}
        {summary.unsaved > 0 && <li>{summary.unsaved} 项修改还在发送或保存中。</li>}
        {summary.attention > 0 && <li>{summary.attention} 项修改需要处理（冲突或被拒绝）。</li>}
      </ul>
      <div className="aiws-dialog-actions">
        <button type="button" className="is-primary" autoFocus data-testid="aiws-leave-stay" onClick={onStay}>返回处理</button>
        <button type="button" data-testid="aiws-leave-export" onClick={onExportAndLeave}>导出后离开</button>
        <button type="button" className="is-danger" data-testid="aiws-leave-anyway" onClick={onLeave}>仍然离开</button>
      </div>
    </Modal>
  )
}
