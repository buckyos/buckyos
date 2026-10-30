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
import { useAiwsText } from '../../useAiwsText'
import { CANVAS_MODE_LABEL } from '../blocks/registry'
import { SURFACE_ICONS, SurfaceIcon } from '../canvas/icons'
import { accepted, renameBase, renameSurface, setSurfaceBackground, setSurfaceIcon, surfaceWriteReason, useSurfaceConfig, type RenameBase } from '../canvas/surfaceManage'
import { buildMainMenu, type CanvasCommands } from './MainMenu'
import { MenuButton, PopoverPanel, usePopover } from './popover'
import { duplicateSurfaceOps } from '../canvas/surfaceOps'
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
  const { t } = useAiwsText()
  useOutlineVersion()
  const mode = useCanvasMode()
  const readOnlyNow = useReadOnlyReason()
  const surface = shell.activeSurface
  const writeReason = surfaceWriteReason(store, surface, mode !== 'edit', readOnlyNow)
  const inSources = shell.topMode === 'sources'
  return (
    <div className="aiws-main-area">
      <div className="aiws-panel aiws-main-toolbar" role="toolbar" aria-label={t('aiworkspace.shell.toolbar.main')} data-testid="aiws-main-toolbar">
        {surface ? (
          <>
            <SurfaceIconButton surface={surface} reason={writeReason} />
            <SurfaceName key={surface.entity_id} surface={surface} reason={writeReason} />
          </>
        ) : (
          <span className="aiws-main-title" data-testid="aiws-surface-name"><LayoutDashboard size={18} aria-hidden="true" /> <b>{store.session.info().title}</b></span>
        )}
        <SurfaceSwitcher />
        {mode !== 'edit' && !inSources && <span className="aiws-chip aiws-mode-chip" data-testid="aiws-mode-note" title={mode === 'view' ? t('aiworkspace.shell.toolbar.viewModeHint') : t('aiworkspace.shell.toolbar.otherModeHint')}>{CANVAS_MODE_LABEL[mode]}</span>}
        <span className="aiws-toolbar-sep" aria-hidden="true" />
        {inSources ? (
          <button type="button" className="aiws-tool aiws-tool-text" aria-pressed="true" data-testid="aiws-top-canvas" title={t('aiworkspace.shell.toolbar.backToCanvasHint')} onClick={() => shell.setTopMode('canvas')}>
            <Database size={18} /><span>{t('aiworkspace.shell.toolbar.backToCanvas')}</span>
          </button>
        ) : (
          <button type="button" className="aiws-tool" aria-label={t('aiworkspace.shell.toolbar.sources')} title={t('aiworkspace.shell.toolbar.sourcesHint')} data-testid="aiws-top-sources" onClick={() => shell.setTopMode('sources')}>
            <Database size={18} />
          </button>
        )}
        <MenuButton label={t('aiworkspace.shell.toolbar.mainMenu')} title={t('aiworkspace.shell.toolbar.mainMenu')} testId="aiws-main-menu" menuTestId="aiws-main-menu-list" items={() => buildMainMenu({ store, shell, canvas, mode })}>
          <Ellipsis size={18} />
        </MenuButton>
        <button type="button" className={`aiws-tool${shell.size === 'wide' ? ' aiws-tool-text' : ''}`} aria-pressed={shell.side === 'collab'} aria-label={t('aiworkspace.shell.side.collab')} title={t('aiworkspace.shell.toolbar.collabHint')} data-testid="aiws-collab"
          onClick={() => shell.setSide(shell.side === 'collab' ? null : 'collab')}>
          <Users size={18} />{shell.size === 'wide' && <span>{t('aiworkspace.shell.side.collab')}</span>}
        </button>
      </div>
    </div>
  )
}

function SurfaceIconButton({ surface, reason }: { surface: EntityEnvelope; reason: string | null }) {
  const store = useStore()
  const { t } = useAiwsText()
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const choose = (icon: string | null) => {
    close()
    if ((surface.icon ?? null) === icon) return
    setSurfaceIcon(store, surface, icon).catch((error: unknown) => store.notify('error', t('aiworkspace.shell.toolbar.iconFailed', { error: describeError(error) })))
  }
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-tool" aria-label={t('aiworkspace.shell.toolbar.icon')} title={reason ?? t('aiworkspace.shell.toolbar.changeIcon')} aria-haspopup="dialog" aria-expanded={open}
        data-testid="aiws-surface-icon" data-icon={surface.icon ?? ''} disabled={reason !== null && !open} onClick={toggle}>
        <SurfaceIcon surface={surface} size={20} />
      </button>
      {open && (
        <PopoverPanel label={t('aiworkspace.shell.toolbar.chooseIcon')} testId="aiws-icon-picker" className="aiws-icon-picker">
          <div className="aiws-icon-grid" role="listbox" aria-label={t('aiworkspace.shell.presetIcons')}>
            {SURFACE_ICONS.map(({ id, label, Icon }) => (
              <button key={id} type="button" role="option" aria-selected={surface.icon === id} aria-label={label} title={label} data-testid={`aiws-icon-${id}`} onClick={() => choose(id)}><Icon size={20} /></button>
            ))}
          </div>
          <button type="button" className="aiws-popover-item" data-testid="aiws-icon-default" onClick={() => choose(null)}>{t('aiworkspace.shell.toolbar.defaultIcon')}</button>
          {surface.layout?.mode === 'free' && <SurfaceBackground surface={surface} />}
        </PopoverPanel>
      )}
    </span>
  )
}

/** The page background of a free canvas (draw.io 导入 TODO §2.5): none (the theme's), a preset or any colour. */
const BACKGROUNDS = [
  { value: '#ffffff', label: 'aiworkspace.shell.background.white' }, { value: '#fffbe6', label: 'aiworkspace.shell.background.cream' },
  { value: '#f5f5f5', label: 'aiworkspace.shell.background.lightGray' }, { value: '#eef5ff', label: 'aiworkspace.shell.background.lightBlue' },
  { value: '#f1f8ee', label: 'aiworkspace.shell.background.lightGreen' }, { value: '#1e1e1e', label: 'aiworkspace.shell.background.dark' },
]

function SurfaceBackground({ surface }: { surface: EntityEnvelope }) {
  const store = useStore()
  const { t } = useAiwsText()
  const { background } = useSurfaceConfig(surface.entity_id)
  const pick = (value: string | null) => {
    if (value === background) return
    setSurfaceBackground(store, surface, value).catch((error: unknown) => store.notify('error', t('aiworkspace.shell.background.failed', { error: describeError(error) })))
  }
  return (
    <div className="aiws-surface-background" data-testid="aiws-surface-background">
      <div className="aiws-muted">{t('aiworkspace.shell.background.title')}</div>
      <div className="aiws-palette-grid">
        <button type="button" className="aiws-palette-swatch is-auto" aria-label={t('aiworkspace.shell.background.theme')} title={t('aiworkspace.shell.background.theme')} aria-pressed={background === null} data-testid="aiws-background-default" onClick={() => pick(null)} />
        {BACKGROUNDS.map((b) => <button key={b.value} type="button" className="aiws-palette-swatch" aria-label={t(b.label)} title={t(b.label)} aria-pressed={background === b.value} style={{ background: b.value }} data-testid={`aiws-background-${b.value.slice(1)}`} onClick={() => pick(b.value)} />)}
      </div>
      <label className="aiws-palette-custom">{t('aiworkspace.shell.background.custom')} <input type="color" aria-label={t('aiworkspace.shell.background.customColor')} defaultValue={background ?? '#ffffff'} onChange={(event) => pick(event.target.value)} /></label>
    </div>
  )
}

/** Click to edit in place: Enter commits, Esc cancels (the blur that follows saves nothing), blur commits a
 * non-empty change; an empty name restores the old one. A refused rename keeps the input and says why. */
function SurfaceName({ surface, reason }: { surface: EntityEnvelope; reason: string | null }) {
  const store = useStore()
  const { t } = useAiwsText()
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
    if (title === '') { setDraft(null); store.notify('info', t('aiworkspace.shell.name.emptyRestored')); return }
    if (title === current) { setDraft(null); return }
    let versions: RenameBase
    try {
      if (!base) throw new Error(t('aiworkspace.shell.name.notEditing'))
      versions = await base
    } catch (failure) {
      setError(t('aiworkspace.shell.cannotReadVersions', { error: describeError(failure) }))
      return
    }
    const outcome = await renameSurface(store, surface, title, versions)
    if (accepted(outcome)) { setDraft(null); setError(null); return }
    setError(outcome.status === 'conflict' || ('code' in outcome && outcome.code === 'REVISION_CONFLICT')
      ? t('aiworkspace.shell.name.conflict')
      : t('aiworkspace.shell.renameRefused', { code: 'code' in outcome ? outcome.code : outcome.status }))
    // a later Enter overwrites knowingly: the next attempt expects the versions as they are now
    setBase(readBase(store, surface))
  }
  if (draft === null) {
    return (
      <button type="button" className="aiws-surface-name" data-testid="aiws-surface-name" title={reason ? t('aiworkspace.shell.name.titleReason', { name: current, reason }) : t('aiworkspace.shell.name.titleRename', { name: current })} onClick={start}>
        <span>{current}</span>
      </button>
    )
  }
  return (
    <span className="aiws-anchor aiws-surface-name-edit">
      <input autoFocus aria-label={t('aiworkspace.shell.name.input')} data-testid="aiws-surface-name-input" value={draft} maxLength={256} aria-invalid={error !== null}
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
  const { t } = useAiwsText()
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const [query, setQuery] = useState('')
  const [renaming, setRenaming] = useState<{ id: string; base: Promise<RenameBase>; value: string } | null>(null)
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
    try { versions = await renaming.base } catch (failure) { store.notify('error', t('aiworkspace.shell.cannotReadVersions', { error: describeError(failure) })); return }
    const outcome = await renameSurface(store, surface, title, versions)
    if (!accepted(outcome)) store.notify('error', t('aiworkspace.shell.renameRefused', { code: 'code' in outcome ? outcome.code : outcome.status }))
  }
  return (
    <span ref={bindAnchor} className={`aiws-anchor${phone ? ' aiws-surface-anchor' : ''}`}>
      {phone ? (
        <button ref={bindTrigger} type="button" className="aiws-surface-trigger" aria-label={t('aiworkspace.shell.switcher.label', { name: active ? active.title ?? active.name ?? '' : workspaceTitle })} title={t('aiworkspace.shell.switchCanvas')} aria-haspopup="dialog" aria-expanded={open}
          data-testid="aiws-surface-switch" data-icon={active?.icon ?? ''} onClick={toggle}>
          {active ? <SurfaceIcon surface={active} size={20} /> : <LayoutDashboard size={20} aria-hidden="true" />}
          <span className="aiws-surface-trigger-name" data-testid="aiws-surface-name">{active ? active.title ?? active.name ?? active.entity_id : workspaceTitle}</span>
          <ChevronDown size={16} aria-hidden="true" />
        </button>
      ) : (
        <button ref={bindTrigger} type="button" className="aiws-tool aiws-tool-narrow" aria-label={t('aiworkspace.shell.switchCanvas')} title={t('aiworkspace.shell.switchCanvas')} aria-haspopup="dialog" aria-expanded={open} data-testid="aiws-surface-switch" onClick={toggle}>
          <ChevronDown size={16} />
        </button>
      )}
      {open && (
        <PopoverPanel label={t('aiworkspace.shell.new.tab.canvas')} testId="aiws-surface-list" className="aiws-surface-list">
          <div className="aiws-surface-list-head">
            <span className="aiws-muted">{t('aiworkspace.shell.new.tab.workspace')}</span>
            <b data-testid="aiws-workspace-title">{workspaceTitle}</b>
            {phone && (
              <>
                <button type="button" className="aiws-popover-item" data-testid="aiws-switch-workspace" onClick={() => { close(); shell.close() }}>
                  <ArrowLeftRight size={16} aria-hidden="true" /> {t('aiworkspace.shell.switcher.switchWorkspace')}
                </button>
                <span className="aiws-muted" data-testid="aiws-phone-note">{t('aiworkspace.shell.switcher.phoneNote')}</span>
              </>
            )}
          </div>
          {shell.surfaces.length > 6 && (
            <label className="aiws-search"><Search size={14} aria-hidden="true" /><input type="search" aria-label={t('aiworkspace.shell.switcher.search')} placeholder={t('aiworkspace.shell.switcher.searchPlaceholder')} value={query} onChange={(event) => setQuery(event.target.value)} /></label>
          )}
          <div className="aiws-surface-items" role="menu" aria-label={t('aiworkspace.shell.switcher.list')}>
            {surfaces.map((surface) => (
              <div key={surface.entity_id} className={`aiws-surface-item${surface.entity_id === shell.activeSurface?.entity_id ? ' is-active' : ''}`} data-testid={`aiws-surface-item-${surface.entity_id}`}>
                {renaming?.id === surface.entity_id ? (
                  <input autoFocus aria-label={t('aiworkspace.shell.switcher.titleInput')} value={renaming.value} onChange={(event) => setRenaming({ ...renaming, value: event.target.value })}
                    onBlur={() => { void rename() }} onKeyDown={(event) => { if (event.key === 'Enter') (event.target as HTMLInputElement).blur(); if (event.key === 'Escape') { event.stopPropagation(); setRenaming(null) } }} />
                ) : (
                  <button type="button" role="menuitem" className="aiws-surface-pick" aria-current={surface.entity_id === shell.activeSurface?.entity_id} onClick={() => { shell.selectSurface(surface.entity_id); shell.setTopMode('canvas'); close() }}>
                    <SurfaceIcon surface={surface} size={16} />
                    <span className="aiws-surface-item-name">{surface.title ?? surface.name ?? surface.entity_id}</span>
                    <span className="aiws-chip">{surface.layout?.mode === 'free' ? t('aiworkspace.shell.freeCanvas') : t('aiworkspace.shell.flowPage')}</span>
                  </button>
                )}
                {!phone && (surface.capabilities.includes('structure') || surface.capabilities.includes('delete')) && (
                  <MenuButton label={t('aiworkspace.shell.switcher.more', { name: surface.title ?? surface.name ?? '' })} className="aiws-tool aiws-tool-small" testId={`aiws-surface-more-${surface.entity_id}`} align="end" floating
                    items={[
                      { id: `rename-${surface.entity_id}`, label: t('aiworkspace.common.rename'), disabled: !surface.capabilities.includes('structure'), reason: t('aiworkspace.shell.switcher.noRenamePermission'),
                        run: () => setRenaming({ id: surface.entity_id, base: readBase(store, surface), value: surface.title ?? surface.name ?? '' }) },
                      { id: `surface-duplicate-${surface.entity_id}`, testId: `aiws-surface-duplicate-${surface.entity_id}`, label: t('aiworkspace.shell.switcher.duplicate'), disabled: !canStructure, reason: t('aiworkspace.shell.switcher.noRenamePermission'),
                        run: () => { void duplicateSurfaceOps(store, surface.entity_id).then((made) => store.submit({ editId: `surface:${made.surfaceId}`, label: t('aiworkspace.shell.switcher.duplicate'), operations: made.ops }).then((outcome) => { if (outcome.status === 'accepted' || outcome.status === 'saved_locally') { shell.selectSurface(made.surfaceId); shell.setTopMode('canvas') } })).catch((error: unknown) => store.notify('error', describeError(error))) } },
                      { id: `surface-delete-${surface.entity_id}`, testId: `aiws-surface-delete-${surface.entity_id}`, label: t('aiworkspace.shell.switcher.delete'), danger: true, disabled: !surface.capabilities.includes('delete'), reason: t('aiworkspace.shell.switcher.noDeletePermission'),
                        run: () => { close(); shell.openDialog({ kind: 'delete-surface', surface }) } },
                    ]}>
                    <Ellipsis size={14} />
                  </MenuButton>
                )}
              </div>
            ))}
            {surfaces.length === 0 && <div className="aiws-muted">{q ? t('aiworkspace.shell.switcher.noMatch') : t('aiworkspace.shell.switcher.empty')}</div>}
          </div>
          {canStructure && <button type="button" className="aiws-popover-item" data-testid="aiws-surface-new" onClick={() => { close(); shell.openDialog({ kind: 'new', tab: 'canvas' }) }}>{t('aiworkspace.shell.switcher.new')}</button>}
        </PopoverPanel>
      )}
    </span>
  )
}
