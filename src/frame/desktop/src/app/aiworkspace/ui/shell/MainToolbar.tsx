/* The main toolbar, top left (UI improvement §5): canvas icon | canvas name + a separate drop-down |
 * data source | main menu (⋯) | collaboration. The save state is in the status area, bottom right. Name editing
 * and switching are separate controls; the icon and the name are shared Surface properties written
 * through the store (permission, version and undo rules apply). A phone has no main toolbar: its one
 * toolbar starts with the view-only canvas switcher (§16). */

import { useState } from 'react'
import { ArrowLeftRight, ChevronDown, Database, Ellipsis, LayoutDashboard, Search, Users } from 'lucide-react'
import type { EntityEnvelope } from '../../api/types'
import { describeError } from '../../api/session'
import { useOutlineVersion, useReadOnlyReason, useStore } from '../../state/hooks'
import { CANVAS_MODE_LABEL } from '../blocks/registry'
import { SURFACE_ICONS, SurfaceIcon } from '../canvas/icons'
import { accepted, renameBase, renameSurface, setSurfaceIcon, SurfaceDeleteDialog, surfaceWriteReason, type RenameBase } from '../canvas/surfaceManage'
import { buildMainMenu, type CanvasCommands } from './MainMenu'
import { MenuButton, PopoverPanel, usePopover } from './popover'
import { useCanvasMode, useShell } from './shellContext'

/** The versions a rename will expect, read when editing starts; a failure surfaces when the rename awaits it. */
function readBase(store: ReturnType<typeof useStore>, surface: EntityEnvelope): Promise<RenameBase> {
  const read = renameBase(store, surface)
  read.catch(() => undefined)
  return read
}

export function MainToolbar({ canvas }: { canvas: CanvasCommands | null }) {
  const store = useStore()
  const shell = useShell()
  useOutlineVersion()
  const mode = useCanvasMode()
  const readOnlyNow = useReadOnlyReason()
  const surface = shell.activeSurface
  const writeReason = surfaceWriteReason(store, surface, mode !== 'edit', readOnlyNow)
  const inSources = shell.topMode === 'sources'
  return (
    <div className="aiws-main-area">
      <div className="aiws-panel aiws-main-toolbar" role="toolbar" aria-label="主工具栏" data-testid="aiws-main-toolbar">
        {surface ? (
          <>
            <SurfaceIconButton surface={surface} reason={writeReason} />
            <SurfaceName key={surface.entity_id} surface={surface} reason={writeReason} />
          </>
        ) : (
          <span className="aiws-main-title" data-testid="aiws-surface-name"><LayoutDashboard size={18} aria-hidden="true" /> <b>{store.session.info().title}</b></span>
        )}
        <SurfaceSwitcher />
        {mode !== 'edit' && !inSources && <span className="aiws-chip aiws-mode-chip" data-testid="aiws-mode-note" title={mode === 'view' ? '查看模式：除批注外不修改文档' : '播放编辑：尚未实现，只读占位'}>{CANVAS_MODE_LABEL[mode]}</span>}
        <span className="aiws-toolbar-sep" aria-hidden="true" />
        {inSources ? (
          <button type="button" className="aiws-tool aiws-tool-text" aria-pressed="true" data-testid="aiws-top-canvas" title="返回之前的画布和视口" onClick={() => shell.setTopMode('canvas')}>
            <Database size={18} /><span>返回画布</span>
          </button>
        ) : (
          <button type="button" className="aiws-tool" aria-label="数据源" title="数据源：数据树、详情、属性与关系" data-testid="aiws-top-sources" onClick={() => shell.setTopMode('sources')}>
            <Database size={18} />
          </button>
        )}
        <MenuButton label="主菜单" title="主菜单" testId="aiws-main-menu" menuTestId="aiws-main-menu-list" items={() => buildMainMenu({ store, shell, canvas, mode })}>
          <Ellipsis size={18} />
        </MenuButton>
        <button type="button" className={`aiws-tool${shell.size === 'wide' ? ' aiws-tool-text' : ''}`} aria-pressed={shell.side === 'collab'} aria-label="协作" title="协作：访问权限与授权" data-testid="aiws-collab"
          onClick={() => shell.setSide(shell.side === 'collab' ? null : 'collab')}>
          <Users size={18} />{shell.size === 'wide' && <span>协作</span>}
        </button>
      </div>
    </div>
  )
}

function SurfaceIconButton({ surface, reason }: { surface: EntityEnvelope; reason: string | null }) {
  const store = useStore()
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const choose = (icon: string | null) => {
    close()
    if ((surface.icon ?? null) === icon) return
    setSurfaceIcon(store, surface, icon).catch((error: unknown) => store.notify('error', `更换图标失败：${describeError(error)}`))
  }
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-tool" aria-label="画布图标" title={reason ?? '更换画布图标'} aria-haspopup="dialog" aria-expanded={open}
        data-testid="aiws-surface-icon" data-icon={surface.icon ?? ''} disabled={reason !== null && !open} onClick={toggle}>
        <SurfaceIcon surface={surface} size={20} />
      </button>
      {open && (
        <PopoverPanel label="选择画布图标" testId="aiws-icon-picker" className="aiws-icon-picker">
          <div className="aiws-icon-grid" role="listbox" aria-label="预设图标">
            {SURFACE_ICONS.map(({ id, label, Icon }) => (
              <button key={id} type="button" role="option" aria-selected={surface.icon === id} aria-label={label} title={label} data-testid={`aiws-icon-${id}`} onClick={() => choose(id)}><Icon size={20} /></button>
            ))}
          </div>
          <button type="button" className="aiws-popover-item" data-testid="aiws-icon-default" onClick={() => choose(null)}>使用默认图标</button>
        </PopoverPanel>
      )}
    </span>
  )
}

/** Click to edit in place: Enter commits, Esc cancels (the blur that follows saves nothing), blur commits a
 * non-empty change; an empty name restores the old one. A refused rename keeps the input and says why. */
function SurfaceName({ surface, reason }: { surface: EntityEnvelope; reason: string | null }) {
  const store = useStore()
  const [draft, setDraft] = useState<string | null>(null)
  const [base, setBase] = useState<Promise<RenameBase> | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [cancelled, setCancelled] = useState(false)
  const current = surface.title ?? surface.name ?? surface.entity_id
  const start = () => {
    if (reason) return
    setDraft(current)
    setError(null)
    setCancelled(false)
    setBase(readBase(store, surface))
  }
  const commit = async () => {
    if (draft === null || cancelled) return
    const title = draft.trim()
    if (title === '') { setDraft(null); store.notify('info', '画布名称不能为空，已恢复原名称。'); return }
    if (title === current) { setDraft(null); return }
    let versions: RenameBase
    try {
      if (!base) throw new Error('没有开始编辑')
      versions = await base
    } catch (failure) {
      setError(`无法读取画布版本：${describeError(failure)}`)
      return
    }
    const outcome = await renameSurface(store, surface, title, versions)
    if (accepted(outcome)) { setDraft(null); setError(null); return }
    setError(outcome.status === 'conflict' || ('code' in outcome && outcome.code === 'REVISION_CONFLICT')
      ? '其他人刚改过这张画布的名称，你的输入保留在此：确认后再按 Enter 覆盖，或按 Esc 放弃。'
      : `改名未被接受：${'code' in outcome ? outcome.code : outcome.status}`)
    // a later Enter overwrites knowingly: the next attempt expects the versions as they are now
    setBase(readBase(store, surface))
  }
  if (draft === null) {
    return (
      <button type="button" className="aiws-surface-name" data-testid="aiws-surface-name" title={reason ? `${current}（${reason}）` : `${current}（点击改名）`} onClick={start}>
        <span>{current}</span>
      </button>
    )
  }
  return (
    <span className="aiws-anchor aiws-surface-name-edit">
      <input autoFocus aria-label="画布名称" data-testid="aiws-surface-name-input" value={draft} maxLength={256} aria-invalid={error !== null}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter') { event.preventDefault(); void commit() }
          if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setCancelled(true); setDraft(null); setError(null) }
        }}
        onBlur={() => { if (!error) void commit() }} />
      {error && <span className="aiws-popover aiws-popover-start aiws-inline-error" role="alert" data-testid="aiws-surface-name-error">{error}</span>}
    </span>
  )
}

/** The canvas drop-down. On a phone (§16) its trigger is the current canvas's icon and name, the list only switches
 * (no rename, delete or new canvas) and its head leads back to the workspace list. */
export function SurfaceSwitcher({ phone = false }: { phone?: boolean }) {
  const store = useStore()
  const shell = useShell()
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const [query, setQuery] = useState('')
  const [renaming, setRenaming] = useState<{ id: string; base: Promise<RenameBase>; value: string } | null>(null)
  const [deleting, setDeleting] = useState<EntityEnvelope | null>(null)
  const canStructure = store.session.info().capabilities.includes('structure') && !phone
  const active = shell.activeSurface
  const workspaceTitle = store.session.info().title
  const q = query.trim().toLowerCase()
  const surfaces = shell.surfaces.filter((s) => !q || (s.title ?? s.name ?? '').toLowerCase().includes(q))
  const rename = async () => {
    if (!renaming) return
    const surface = store.outline.get(renaming.id)
    const title = renaming.value.trim()
    setRenaming(null)
    if (!surface || !title || title === (surface.title ?? surface.name)) return
    let versions: RenameBase
    try { versions = await renaming.base } catch (failure) { store.notify('error', `无法读取画布版本：${describeError(failure)}`); return }
    const outcome = await renameSurface(store, surface, title, versions)
    if (!accepted(outcome)) store.notify('error', `改名未被接受：${'code' in outcome ? outcome.code : outcome.status}`)
  }
  return (
    <span ref={bindAnchor} className={`aiws-anchor${phone ? ' aiws-surface-anchor' : ''}`}>
      {phone ? (
        <button ref={bindTrigger} type="button" className="aiws-surface-trigger" aria-label={`切换画布：${active ? active.title ?? active.name ?? '' : workspaceTitle}`} title="切换画布" aria-haspopup="dialog" aria-expanded={open}
          data-testid="aiws-surface-switch" data-icon={active?.icon ?? ''} onClick={toggle}>
          {active ? <SurfaceIcon surface={active} size={20} /> : <LayoutDashboard size={20} aria-hidden="true" />}
          <span className="aiws-surface-trigger-name" data-testid="aiws-surface-name">{active ? active.title ?? active.name ?? active.entity_id : workspaceTitle}</span>
          <ChevronDown size={16} aria-hidden="true" />
        </button>
      ) : (
        <button ref={bindTrigger} type="button" className="aiws-tool aiws-tool-narrow" aria-label="切换画布" title="切换画布" aria-haspopup="dialog" aria-expanded={open} data-testid="aiws-surface-switch" onClick={toggle}>
          <ChevronDown size={16} />
        </button>
      )}
      {open && (
        <PopoverPanel label="画布" testId="aiws-surface-list" className="aiws-surface-list">
          <div className="aiws-surface-list-head">
            <span className="aiws-muted">工作区</span>
            <b data-testid="aiws-workspace-title">{workspaceTitle}</b>
            {phone && (
              <>
                <button type="button" className="aiws-popover-item" data-testid="aiws-switch-workspace" onClick={() => { close(); shell.close() }}>
                  <ArrowLeftRight size={16} aria-hidden="true" /> 切换工作区
                </button>
                <span className="aiws-muted" data-testid="aiws-phone-note">手机上只能查看；编辑请在电脑上打开。</span>
              </>
            )}
          </div>
          {shell.surfaces.length > 6 && (
            <label className="aiws-search"><Search size={14} aria-hidden="true" /><input type="search" aria-label="搜索画布" placeholder="搜索画布…" value={query} onChange={(event) => setQuery(event.target.value)} /></label>
          )}
          <div className="aiws-surface-items" role="menu" aria-label="画布列表">
            {surfaces.map((surface) => (
              <div key={surface.entity_id} className={`aiws-surface-item${surface.entity_id === shell.activeSurface?.entity_id ? ' is-active' : ''}`} data-testid={`aiws-surface-item-${surface.entity_id}`}>
                {renaming?.id === surface.entity_id ? (
                  <input autoFocus aria-label="画布标题" value={renaming.value} onChange={(event) => setRenaming({ ...renaming, value: event.target.value })}
                    onBlur={() => { void rename() }} onKeyDown={(event) => { if (event.key === 'Enter') (event.target as HTMLInputElement).blur(); if (event.key === 'Escape') { event.stopPropagation(); setRenaming(null) } }} />
                ) : (
                  <button type="button" role="menuitem" className="aiws-surface-pick" aria-current={surface.entity_id === shell.activeSurface?.entity_id} onClick={() => { shell.selectSurface(surface.entity_id); shell.setTopMode('canvas'); close() }}>
                    <SurfaceIcon surface={surface} size={16} />
                    <span className="aiws-surface-item-name">{surface.title ?? surface.name ?? surface.entity_id}</span>
                    <span className="aiws-chip">{surface.layout?.mode === 'free' ? '自由画布' : '流式页'}</span>
                  </button>
                )}
                {!phone && (surface.capabilities.includes('structure') || surface.capabilities.includes('delete')) && (
                  <MenuButton label={`「${surface.title ?? surface.name ?? ''}」的更多操作`} className="aiws-tool aiws-tool-small" testId={`aiws-surface-more-${surface.entity_id}`} align="end"
                    items={[
                      { id: `rename-${surface.entity_id}`, label: '重命名', disabled: !surface.capabilities.includes('structure'), reason: '没有修改这张画布的权限',
                        run: () => setRenaming({ id: surface.entity_id, base: readBase(store, surface), value: surface.title ?? surface.name ?? '' }) },
                      { id: `surface-delete-${surface.entity_id}`, testId: `aiws-surface-delete-${surface.entity_id}`, label: '删除画布…', danger: true, disabled: !surface.capabilities.includes('delete'), reason: '没有删除这张画布的权限', run: () => setDeleting(surface) },
                    ]}>
                    <Ellipsis size={14} />
                  </MenuButton>
                )}
              </div>
            ))}
            {surfaces.length === 0 && <div className="aiws-muted">{q ? '没有匹配的画布' : '还没有画布'}</div>}
          </div>
          {canStructure && <button type="button" className="aiws-popover-item" data-testid="aiws-surface-new" onClick={() => { close(); shell.openDialog({ kind: 'new', tab: 'canvas' }) }}>＋ 新建画布</button>}
        </PopoverPanel>
      )}
      {deleting && <SurfaceDeleteDialog surface={deleting} onClose={() => setDeleting(null)} onDeleted={(next) => { if (next) shell.selectSurface(next) }} />}
    </span>
  )
}
