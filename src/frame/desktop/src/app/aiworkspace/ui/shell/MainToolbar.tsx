/* The title zone, top left (UI 框架 PRD §4.1): canvas icon | canvas name + a separate drop-down | mode chip | data
 * mode | main menu (⋯). Online members are reached from the collaboration zone's avatars, permissions from
 * "成员与权限…". The save state is in the status bar, bottom right. Name editing and switching are separate controls; the
 * icon and the name are shared Surface properties written through the store (permission, version and undo rules
 * apply). A phone's title zone (§6.1) is its own: the canvas icon and name ▾ (the picker), undo and redo while a flow
 * page is edited, and its own main menu ⋯ — the way to everything else a phone does. */

import { useEffect, useState } from 'react'
import { Database, Ellipsis, LayoutDashboard } from 'lucide-react'
import type { EntityEnvelope } from '../../api/types'
import { describeError } from '../../api/session'
import { useOutlineVersion, useReadOnlyReason, useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { CANVAS_MODE_LABEL } from '../blocks/registry'
import { SURFACE_ICONS, SurfaceIcon } from '../canvas/icons'
import { accepted, renameBase, renameSurface, setSurfaceBackground, setSurfaceIcon, surfaceWriteReason, useSurfaceConfig, type RenameBase } from '../canvas/surfaceManage'
import { useActionContext } from '../framework/hooks'
import { useMacroWatch } from '../framework/macros'
import { onPageSettings } from '../framework/pageSettings'
import { buildMenu } from '../framework/actions'
import { HistoryButtons } from '../canvas/ObjectToolbar'
import { buildMainMenu, type CanvasCommands } from './MainMenu'
import { MenuButton, PopoverPanel, usePopover } from './popover'
import { useCanvasMode, useShell } from './shellContext'
import { SurfacePicker } from '../surfaces/SurfacePicker'
import '../phone/phone.css'

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
  const actions = useActionContext(canvas)
  useMacroWatch()
  // Q4: data mode keeps one icon in the title zone; inside data mode the same switch leads back to the canvas
  const dataLabel = inSources ? t('aiworkspace.shell.toolbar.backToCanvas') : t('aiworkspace.framework.dataMode')
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
        <SurfacePicker />
        {mode !== 'edit' && !inSources && <span className="aiws-chip aiws-mode-chip" data-testid="aiws-mode-note" title={mode === 'view' ? t('aiworkspace.shell.toolbar.viewModeHint') : t('aiworkspace.shell.toolbar.otherModeHint')}>{CANVAS_MODE_LABEL[mode]}</span>}
        <span className="aiws-toolbar-sep" aria-hidden="true" />
        <button type="button" className="aiws-tool" aria-pressed={inSources} aria-label={dataLabel} title={inSources ? t('aiworkspace.shell.toolbar.backToCanvasHint') : t('aiworkspace.framework.dataModeHint')}
          data-testid={inSources ? 'aiws-top-canvas' : 'aiws-top-sources'} data-action="view.data-mode" onClick={() => shell.setTopMode(inSources ? 'canvas' : 'sources')}>
          <Database size={18} />
        </button>
        <MenuButton label={t('aiworkspace.shell.toolbar.mainMenu')} title={t('aiworkspace.shell.toolbar.mainMenu')} testId="aiws-main-menu" menuTestId="aiws-main-menu-list" items={() => buildMainMenu(actions)}>
          <Ellipsis size={18} />
        </MenuButton>
      </div>
    </div>
  )
}

/** The phone's title zone (UI 框架 PRD §6.1): the canvas icon and name ▾ — the whole title opens the picker, full screen —,
 * undo and redo while a flow page is edited, the phone's main menu ⋯ (only what a phone can do, from the bottom). The page
 * settings ("画布 ▸ 页面设置…") open from the menu: an explicit way in, so they follow the rights, not the view mode. */
export function PhoneTitle({ canvas }: { canvas: CanvasCommands | null }) {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  useOutlineVersion()
  const mode = useCanvasMode()
  const readOnlyNow = useReadOnlyReason()
  const actions = useActionContext(canvas)
  useMacroWatch()
  const surface = shell.activeSurface
  return (
    <div className="aiws-main-area aiws-phone-title-area">
      <div className="aiws-panel aiws-phone-title" role="toolbar" aria-label={t('aiworkspace.shell.toolbar.main')} data-testid="aiws-phone-title">
        <SurfacePicker phone />
        {surface && <SurfaceIconButton surface={surface} reason={surfaceWriteReason(store, surface, false, readOnlyNow)} menuOnly />}
        {surface && mode === 'edit' && shell.topMode === 'canvas' && <HistoryButtons size={18} />}
        <MenuButton label={t('aiworkspace.shell.toolbar.mainMenu')} title={t('aiworkspace.shell.toolbar.mainMenu')} testId="aiws-main-menu" menuTestId="aiws-main-menu-list" align="end" items={() => buildMenu(actions, { phone: true })}>
          <Ellipsis size={20} />
        </MenuButton>
      </div>
    </div>
  )
}

/** `menuOnly`: no button of its own (a phone's title is the picker): "页面设置…" opens it. */
function SurfaceIconButton({ surface, reason, menuOnly = false }: { surface: EntityEnvelope; reason: string | null; menuOnly?: boolean }) {
  const store = useStore()
  const { t } = useAiwsText()
  const { open, setOpen, toggle, close, bindAnchor, bindTrigger } = usePopover()
  // "画布 ▸ 页面设置…" opens this popover (the icon and the page background)
  useEffect(() => onPageSettings(() => { if (reason === null) setOpen(true) }), [reason, setOpen])
  const choose = (icon: string | null) => {
    close()
    if ((surface.icon ?? null) === icon) return
    setSurfaceIcon(store, surface, icon).catch((error: unknown) => store.notify('error', t('aiworkspace.shell.toolbar.iconFailed', { error: describeError(error) })))
  }
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-tool" aria-label={t('aiworkspace.shell.toolbar.icon')} title={reason ?? t('aiworkspace.shell.toolbar.changeIcon')} aria-haspopup="dialog" aria-expanded={open}
        data-testid="aiws-surface-icon" data-icon={surface.icon ?? ''} disabled={reason !== null && !open} hidden={menuOnly} onClick={toggle}>
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

export { SurfacePicker as SurfaceSwitcher }
