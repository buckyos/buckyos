/* Shell dialogs (UI improvement §6.2, §10, §11): the unified "New" dialog (canvas / workspace /
 * template), export, import, keyboard help, the developer Mock panel and the leave check. The New
 * dialog also serves the workspace list, where only the workspace and template tabs exist. */

import { useState, type ReactNode } from 'react'
import { unwrap, type AiwsClient } from '../../api/client'
import { createDemoWorkspace, TemplateFailure } from '../../api/demos'
import { createSampleWorkspace } from '../../api/sample'
import { describeError, uploadWorkspacePackage } from '../../api/session'
import type { ExportResult, WorkspaceSummary } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import type { AiwsText } from '../../i18n'
import { useAiwsText } from '../../useAiwsText'
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

/** Text keys: the title, description and preview are read in the current language when shown. */
const TEMPLATES = [
  { id: 'sample' as const, title: 'aiworkspace.shell.template.sample.title', description: 'aiworkspace.shell.template.sample.description', preview: 'aiworkspace.shell.template.sample.preview', mock: false },
  { id: 'quarterly' as const, title: 'aiworkspace.shell.template.quarterly.title', description: 'aiworkspace.shell.template.quarterly.description', preview: 'aiworkspace.shell.template.quarterly.preview', mock: true },
  { id: 'film' as const, title: 'aiworkspace.shell.template.film.title', description: 'aiworkspace.shell.template.film.description', preview: 'aiworkspace.shell.template.film.preview', mock: true },
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
  const { t } = useAiwsText()
  const canCanvas = Boolean(store?.session.info().capabilities.includes('structure'))
  const [tab, setTab] = useState<NewTab>(initialTab === 'canvas' && !canCanvas ? 'workspace' : initialTab)
  const tabs: NewTab[] = store ? ['canvas', 'workspace', 'template'] : ['workspace', 'template']
  const label: Record<NewTab, string> = { canvas: t('aiworkspace.shell.new.tab.canvas'), workspace: t('aiworkspace.shell.new.tab.workspace'), template: t('aiworkspace.shell.new.tab.template') }
  return (
    <Modal label={t('aiworkspace.shell.new.title')} testId="aiws-new-dialog" onClose={onClose} className="aiws-new-dialog">
      <div className="aiws-dialog-head"><b>{t('aiworkspace.shell.new.title')}</b><span className="aiws-grow" /><button type="button" className="aiws-link" onClick={onClose}>{t('aiworkspace.common.close')}</button></div>
      <div className="aiws-tabs" role="tablist" aria-label={t('aiworkspace.shell.new.tabs')}>
        {tabs.map((item) => <button key={item} type="button" role="tab" aria-selected={tab === item} data-testid={`aiws-new-tab-${item}`} disabled={item === 'canvas' && !canCanvas} title={item === 'canvas' && !canCanvas ? t('aiworkspace.shell.noStructurePermission') : undefined} onClick={() => setTab(item)}>{label[item]}</button>)}
      </div>
      {tab === 'canvas' && store && <NewCanvas store={store} onDone={(id) => { onClose(); onCreatedSurface?.(id) }} />}
      {tab === 'workspace' && <NewWorkspace client={client} onOpen={(id) => { onClose(); onOpenWorkspace(id) }} />}
      {tab === 'template' && <NewFromTemplate client={client} onOpen={(id) => { onClose(); onOpenWorkspace(id) }} />}
    </Modal>
  )
}

function NewCanvas({ store, onDone }: { store: WorkspaceStore; onDone: (surfaceId: string) => void }) {
  const { t } = useAiwsText()
  const [title, setTitle] = useState('')
  const [layout, setLayout] = useState<'free' | 'flow'>('free')
  const [icon, setIcon] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const create = async () => {
    const name = title.trim() || t('aiworkspace.shell.new.canvas.defaultName', { n: surfacesOf(store).length + 1 })
    setBusy(true)
    setError(null)
    const { ops, surfaceId } = createSurfaceOps(store, name, layout, icon)
    const outcome = await store.submit({ editId: 'surface:new', label: t('aiworkspace.shell.new.canvas.editLabel', { name }), operations: ops })
    setBusy(false)
    if (accepted(outcome)) onDone(surfaceId)
    else setError(t('aiworkspace.shell.new.canvas.notCreated', { code: 'code' in outcome ? outcome.code : outcome.status }))
  }
  return (
    <form className="aiws-form" onSubmit={(event) => { event.preventDefault(); void create() }}>
      <label className="aiws-field">{t('aiworkspace.shell.new.name')}<input autoFocus aria-label={t('aiworkspace.shell.new.canvas.titleInput')} placeholder={t('aiworkspace.shell.new.canvas.defaultName', { n: surfacesOf(store).length + 1 })} value={title} maxLength={256} onChange={(event) => setTitle(event.target.value)} /></label>
      <fieldset className="aiws-field">
        <legend>{t('aiworkspace.shell.new.canvas.layout')}</legend>
        <label><input type="radio" name="aiws-new-layout" checked={layout === 'free'} onChange={() => setLayout('free')} /> {t('aiworkspace.shell.new.canvas.layoutFree')}</label>
        <label><input type="radio" name="aiws-new-layout" checked={layout === 'flow'} onChange={() => setLayout('flow')} data-testid="aiws-new-flow" /> {t('aiworkspace.shell.new.canvas.layoutFlow')}</label>
      </fieldset>
      <fieldset className="aiws-field">
        <legend>{t('aiworkspace.shell.new.canvas.icon')}</legend>
        <div className="aiws-icon-grid" role="listbox" aria-label={t('aiworkspace.shell.presetIcons')}>
          {SURFACE_ICONS.map(({ id, label, Icon }) => <button key={id} type="button" role="option" aria-selected={icon === id} aria-label={label} title={label} onClick={() => setIcon(icon === id ? null : id)}><Icon size={18} /></button>)}
        </div>
      </fieldset>
      <div className="aiws-muted">{t('aiworkspace.shell.new.canvas.note')}</div>
      {error && <div className="aiws-error" role="alert">{error}</div>}
      <div className="aiws-dialog-actions"><button type="submit" className="is-primary" data-testid="aiws-surface-create" disabled={busy}>{t('aiworkspace.shell.new.canvas.create')}</button></div>
    </form>
  )
}

function NewWorkspace({ client, onOpen }: { client: AiwsClient; onOpen: (workspaceId: string) => void }) {
  const { t } = useAiwsText()
  const [title, setTitle] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const create = async () => {
    setBusy(true)
    setError(null)
    try {
      const created = unwrap(await client.wsCreate(title.trim() || t('aiworkspace.shell.new.workspace.defaultName')))
      onOpen(created.workspace_id)
    } catch (failure) {
      setError(t('aiworkspace.shell.new.createFailed', { error: describeError(failure) }))
      setBusy(false)
    }
  }
  return (
    <form className="aiws-form" onSubmit={(event) => { event.preventDefault(); void create() }}>
      <label className="aiws-field">{t('aiworkspace.shell.new.name')}<input autoFocus aria-label={t('aiworkspace.shell.new.workspace.titleInput')} placeholder={t('aiworkspace.shell.new.workspace.defaultName')} value={title} onChange={(event) => setTitle(event.target.value)} /></label>
      <div className="aiws-muted">{t('aiworkspace.shell.new.workspace.note')}</div>
      {error && <div className="aiws-error" role="alert" data-testid="aiws-new-error">{error}</div>}
      <div className="aiws-dialog-actions"><button type="submit" className="is-primary" data-testid="aiws-create" disabled={busy}>{busy ? t('aiworkspace.shell.new.workspace.creating') : t('aiworkspace.shell.new.workspace.create')}</button></div>
    </form>
  )
}

function NewFromTemplate({ client, onOpen }: { client: AiwsClient; onOpen: (workspaceId: string) => void }) {
  const { t, tNode } = useAiwsText()
  const [chosen, setChosen] = useState<(typeof TEMPLATES)[number]['id']>('sample')
  const [title, setTitle] = useState('')
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<{ text: string; workspace: WorkspaceSummary | null } | null>(null)
  const template = TEMPLATES.find((item) => item.id === chosen) ?? TEMPLATES[0]
  const create = async () => {
    setBusy(true)
    setFailure(null)
    const name = title.trim() || t(template.title)
    try {
      const created = template.id === 'sample' ? await createSampleWorkspace(client, name) : await createDemoWorkspace(client, template.id, name)
      onOpen(created.workspace_id)
    } catch (error) {
      setFailure(error instanceof TemplateFailure ? { text: t('aiworkspace.shell.new.template.incomplete', { error: error.message }), workspace: error.workspace } : { text: t('aiworkspace.shell.new.createFailed', { error: describeError(error) }), workspace: null })
      setBusy(false)
    }
  }
  const removePartial = async (workspace: WorkspaceSummary) => {
    try { unwrap(await client.wsDelete({ workspace_id: workspace.workspace_id })); setFailure({ text: t('aiworkspace.shell.new.template.partialDeleted', { name: workspace.title }), workspace: null }) } catch (error) { setFailure({ text: t('aiworkspace.shell.new.template.deleteFailed', { error: describeError(error) }), workspace }) }
  }
  return (
    <form className="aiws-form" onSubmit={(event) => { event.preventDefault(); void create() }}>
      <div className="aiws-template-list" role="radiogroup" aria-label={t('aiworkspace.shell.new.tab.template')}>
        {TEMPLATES.map((item) => (
          <label key={item.id} className={`aiws-template${chosen === item.id ? ' is-active' : ''}`} data-testid={`aiws-template-${item.id}`}>
            <input type="radio" name="aiws-template" checked={chosen === item.id} onChange={() => setChosen(item.id)} />
            <span className="aiws-template-text">
              <span className="aiws-template-title"><b>{t(item.title)}</b>{item.mock && <span className="aiws-chip aiws-chip-derived" title={t('aiworkspace.shell.new.template.mockHint')}>{t('aiworkspace.shell.new.template.mock')}</span>}</span>
              <span>{t(item.description)}</span>
              <span className="aiws-muted">{t(item.preview)}</span>
            </span>
          </label>
        ))}
      </div>
      <label className="aiws-field">{t('aiworkspace.shell.new.template.name')}<input aria-label={t('aiworkspace.shell.new.workspace.titleInput')} placeholder={t(template.title)} value={title} onChange={(event) => setTitle(event.target.value)} /></label>
      <div className="aiws-muted">{tNode('aiworkspace.shell.new.template.note', { workspace: <b>{t('aiworkspace.shell.new.template.noteWorkspace')}</b> })}</div>
      {failure && (
        <div className="aiws-error" role="alert" data-testid="aiws-template-failure">
          {failure.text}
          {failure.workspace && (
            <>
              <button type="button" onClick={() => onOpen(failure.workspace?.workspace_id ?? '')}>{t('aiworkspace.shell.new.template.openPartial')}</button>
              <button type="button" onClick={() => { if (failure.workspace) void removePartial(failure.workspace) }}>{t('aiworkspace.shell.new.template.deletePartial')}</button>
            </>
          )}
        </div>
      )}
      <div className="aiws-dialog-actions"><button type="submit" className="is-primary" data-testid="aiws-create-template" disabled={busy}>{busy ? t('aiworkspace.shell.new.template.creating') : t('aiworkspace.shell.new.template.create')}</button></div>
    </form>
  )
}

// ---- export / import

export function ExportForm({ client, workspace, onDone }: { client: AiwsClient; workspace: { workspace_id: string; title: string }; onDone?: (message: string) => void }) {
  const { t } = useAiwsText()
  const [mode, setMode] = useState<'share' | 'personal_backup'>('share')
  const [selfContained, setSelfContained] = useState(true)
  // speaker notes travel in a personal backup, not in what goes to other people (第三期规划 §6.4)
  const [notesChoice, setNotesChoice] = useState<boolean | null>(null)
  const notes = notesChoice ?? mode === 'personal_backup'
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<{ kind: 'info' | 'error'; text: string } | null>(null)
  const [exported, setExported] = useState<ExportResult | null>(null)
  const run = async () => {
    setBusy(true)
    setMessage(null)
    try {
      const result = unwrap(await client.export({ workspace_id: workspace.workspace_id }, mode, selfContained, notes))
      const blob = await client.transport.fetchDownload(result.download)
      saveBlob(blob, `${workspace.title}-${mode === 'share' ? 'share' : 'backup'}.zip`)
      setExported(result)
      const text = t('aiworkspace.shell.export.done', { name: workspace.title, size: blob.size })
      setMessage({ kind: 'info', text })
      onDone?.(text)
    } catch (error) {
      setMessage({ kind: 'error', text: t('aiworkspace.shell.export.failed', { error: describeError(error) }) })
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="aiws-inline-form">
      <select aria-label={t('aiworkspace.shell.export.mode')} value={mode} onChange={(event) => { setMode(event.target.value === 'personal_backup' ? 'personal_backup' : 'share'); setNotesChoice(null) }}>
        <option value="share">{t('aiworkspace.shell.export.modeShare')}</option>
        <option value="personal_backup">{t('aiworkspace.shell.export.modeBackup')}</option>
      </select>
      <label><input type="checkbox" checked={selfContained} onChange={(event) => setSelfContained(event.target.checked)} /> {t('aiworkspace.shell.export.selfContained')}</label>
      <label title={t('aiworkspace.shell.export.notesHint')}><input type="checkbox" data-testid="aiws-export-notes" checked={notes} onChange={(event) => setNotesChoice(event.target.checked)} /> {t('aiworkspace.shell.export.notes')}</label>
      <button type="button" data-testid="aiws-export" disabled={busy} onClick={() => { void run() }}>{busy ? t('aiworkspace.shell.export.exporting') : t('aiworkspace.shell.export.run')}</button>
      {message && <div className={message.kind === 'error' ? 'aiws-error' : 'aiws-muted'} role={message.kind === 'error' ? 'alert' : 'status'} data-testid="aiws-export-message">{message.text}</div>}
      {exported && (
        <div className="aiws-muted" data-testid="aiws-export-manifest">
          {t('aiworkspace.shell.export.last', { mode: exported.manifest.export_mode === 'share' ? t('aiworkspace.shell.export.share') : t('aiworkspace.shell.export.backup'), root: exported.manifest.content_root })}
          {exported.manifest.missing.length > 0 && ` · ${t('aiworkspace.shell.export.missing', { count: exported.manifest.missing.length })}`}
          {exported.manifest.excluded_entities > 0 && ` · ${t('aiworkspace.shell.export.excluded', { count: exported.manifest.excluded_entities })}`}
        </div>
      )}
    </div>
  )
}

export function ImportForm({ client, onDone }: { client: AiwsClient; onDone: (message: string, workspaceId: string) => void }) {
  const { t } = useAiwsText()
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
      const pkg = await uploadWorkspacePackage(client, file)
      const result = unwrap(await client.wsImport(pkg, semantics, semantics === 'restore' && replace))
      setFile(null)
      onDone(semantics === 'new' ? t('aiworkspace.shell.import.doneNew', { id: result.workspace_id }) : t('aiworkspace.shell.import.doneRestore', { id: result.workspace_id }), result.workspace_id)
    } catch (failure) {
      setError(t('aiworkspace.shell.import.failed', { error: describeError(failure) }))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form className="aiws-import" data-testid="aiws-import" onSubmit={(event) => { event.preventDefault(); void run() }}>
      <div className="aiws-panel-title">{t('aiworkspace.shell.import.title')}</div>
      <input type="file" aria-label={t('aiworkspace.shell.import.file')} accept=".zip,application/zip" data-testid="aiws-import-file" onChange={(event) => setFile(event.target.files?.[0] ?? null)} />
      <fieldset>
        <legend>{t('aiworkspace.shell.import.semantics')}</legend>
        <label><input type="radio" name="aiws-semantics" value="new" checked={semantics === 'new'} onChange={() => setSemantics('new')} /> {t('aiworkspace.shell.import.asNew')}</label>
        <label><input type="radio" name="aiws-semantics" value="restore" checked={semantics === 'restore'} onChange={() => setSemantics('restore')} /> {t('aiworkspace.shell.import.asRestore')}</label>
        {semantics === 'restore' && <label><input type="checkbox" checked={replace} onChange={(event) => setReplace(event.target.checked)} /> {t('aiworkspace.shell.import.replace')}</label>}
      </fieldset>
      {error && <div className="aiws-error" role="alert" data-testid="aiws-import-error">{error}</div>}
      <button type="submit" data-testid="aiws-import-submit" disabled={!file || semantics === '' || busy}>{busy ? t('aiworkspace.shell.import.importing') : t('aiworkspace.shell.import.run')}</button>
    </form>
  )
}

export function ExportDialog({ client, workspace, onClose }: { client: AiwsClient; workspace: { workspace_id: string; title: string }; onClose: () => void }) {
  const { t } = useAiwsText()
  return (
    <Modal label={t('aiworkspace.shell.export.dialog')} testId="aiws-export-dialog" onClose={onClose}>
      <div className="aiws-dialog-head"><b>{t('aiworkspace.shell.export.dialogTitle', { name: workspace.title })}</b><span className="aiws-grow" /><button type="button" className="aiws-link" onClick={onClose}>{t('aiworkspace.common.close')}</button></div>
      <div className="aiws-muted">{t('aiworkspace.shell.export.explain')}</div>
      <ExportForm client={client} workspace={workspace} />
    </Modal>
  )
}

export function ImportDialog({ client, onClose, onOpenWorkspace }: { client: AiwsClient; onClose: () => void; onOpenWorkspace: (workspaceId: string) => void }) {
  const { t } = useAiwsText()
  const [done, setDone] = useState<{ text: string; workspaceId: string } | null>(null)
  return (
    <Modal label={t('aiworkspace.shell.import.title')} testId="aiws-import-dialog" onClose={onClose}>
      <div className="aiws-dialog-head"><b>{t('aiworkspace.shell.import.title')}</b><span className="aiws-grow" /><button type="button" className="aiws-link" onClick={onClose}>{t('aiworkspace.common.close')}</button></div>
      {done ? (
        <>
          <div role="status">{done.text}</div>
          <div className="aiws-dialog-actions"><button type="button" className="is-primary" onClick={() => { onClose(); onOpenWorkspace(done.workspaceId) }}>{t('aiworkspace.shell.import.open')}</button><button type="button" onClick={onClose}>{t('aiworkspace.shell.import.stay')}</button></div>
        </>
      ) : <ImportForm client={client} onDone={(text, workspaceId) => setDone({ text, workspaceId })} />}
    </Modal>
  )
}

// ---- help, developer tools, leave check

/** The keyboard help rows: keys (translated where they contain words) and what they do. */
const shortcuts = (t: AiwsText['t']): [string, string][] => [
  ['Ctrl/⌘ + Z', t('aiworkspace.shell.help.undo')], [t('aiworkspace.shell.help.redoKeys'), t('aiworkspace.shell.help.redo')],
  ['Ctrl/⌘ + C / X / V', t('aiworkspace.shell.help.clipboard')], ['Delete / Backspace', t('aiworkspace.shell.help.delete')],
  ['Enter', t('aiworkspace.shell.help.edit')], ['Esc', t('aiworkspace.shell.help.escape')], [t('aiworkspace.shell.help.arrowKeys'), t('aiworkspace.shell.help.move')],
  [t('aiworkspace.shell.help.groupKeys'), t('aiworkspace.shell.help.group')], ['F2', t('aiworkspace.shell.help.properties')], ['V / H', t('aiworkspace.shell.help.tools')],
  [t('aiworkspace.shell.help.panKeys'), t('aiworkspace.shell.help.pan')], [t('aiworkspace.shell.help.zoomKeys'), t('aiworkspace.shell.help.zoom')], [t('aiworkspace.shell.help.scrollKeys'), t('aiworkspace.shell.help.scroll')],
  [t('aiworkspace.shell.help.showNextKeys'), t('aiworkspace.shell.help.showNext')], [t('aiworkspace.shell.help.showPrevKeys'), t('aiworkspace.shell.help.showPrev')], [t('aiworkspace.shell.help.showEndsKeys'), t('aiworkspace.shell.help.showEnds')],
  [t('aiworkspace.shell.help.showBlackKeys'), t('aiworkspace.shell.help.showBlack')], [t('aiworkspace.shell.help.showEscKeys'), t('aiworkspace.shell.help.showEsc')],
  [t('aiworkspace.shell.help.guideKeys'), t('aiworkspace.shell.help.guide')], [t('aiworkspace.shell.help.pathKeys'), t('aiworkspace.shell.help.path')],
]

export function HelpDialog({ onClose }: { onClose: () => void }) {
  const { t } = useAiwsText()
  return (
    <Modal label={t('aiworkspace.shell.help.title')} testId="aiws-help-dialog" onClose={onClose}>
      <div className="aiws-dialog-head"><b>{t('aiworkspace.shell.help.title')}</b><span className="aiws-grow" /><button type="button" className="aiws-link" autoFocus onClick={onClose}>{t('aiworkspace.common.close')}</button></div>
      <table className="aiws-shortcuts"><tbody>{shortcuts(t).map(([keys, text]) => <tr key={keys}><th scope="row"><kbd>{keys}</kbd></th><td>{text}</td></tr>)}</tbody></table>
      <div className="aiws-muted">{t('aiworkspace.shell.help.note')}</div>
    </Modal>
  )
}

export function MockDialog({ onClose }: { onClose: () => void }) {
  const { t } = useAiwsText()
  return (
    <Modal label={t('aiworkspace.shell.mock.dialog')} testId="aiws-mock-dialog" onClose={onClose} modal={false} className="aiws-floating-dialog">
      <div className="aiws-dialog-head"><b>{t('aiworkspace.shell.devTools')}</b><span className="aiws-grow" /><button type="button" className="aiws-link" onClick={onClose}>{t('aiworkspace.common.close')}</button></div>
      <MockRunPanel />
    </Modal>
  )
}

export interface LeaveSummary { unsaved: number; attention: number; memoryOnly: number }

export function LeaveDialog({ summary, reason, onStay, onExportAndLeave, onLeave }: { summary: LeaveSummary; reason: string; onStay: () => void; onExportAndLeave: () => void; onLeave: () => void }) {
  const { t } = useAiwsText()
  return (
    <Modal label={t('aiworkspace.shell.leave.dialog')} testId="aiws-leave-dialog" onClose={onStay}>
      <b>{t('aiworkspace.shell.leave.title', { reason })}</b>
      <ul>
        {summary.memoryOnly > 0 && <li>{t('aiworkspace.shell.leave.memoryOnly', { count: summary.memoryOnly })}</li>}
        {summary.unsaved > 0 && <li>{t('aiworkspace.shell.leave.unsaved', { count: summary.unsaved })}</li>}
        {summary.attention > 0 && <li>{t('aiworkspace.shell.leave.attention', { count: summary.attention })}</li>}
      </ul>
      <div className="aiws-dialog-actions">
        <button type="button" className="is-primary" autoFocus data-testid="aiws-leave-stay" onClick={onStay}>{t('aiworkspace.shell.leave.stay')}</button>
        <button type="button" data-testid="aiws-leave-export" onClick={onExportAndLeave}>{t('aiworkspace.shell.leave.exportAndLeave')}</button>
        <button type="button" className="is-danger" data-testid="aiws-leave-anyway" onClick={onLeave}>{t('aiworkspace.shell.leave.anyway')}</button>
      </div>
    </Modal>
  )
}
