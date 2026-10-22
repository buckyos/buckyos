/* The view toolbar, top right (UI improvement §8.1, §9.1; renamed from the "presenter toolbar" in 第三期规划 §2, which
 * keeps "提示器" for the show's prompter): add annotation | guide | show | zoom ▾ | identity and online members | share
 * link. Zoom and view navigation live only here (and in the main menu): there is no bottom-right navigation area.
 * Interactions are not shown — there is no global interaction system yet — and online members only appear once
 * presence exists. The guide (§7.3) appears when the workspace has a guide path, the show button (§7.2) when it has a
 * presentation path. On a phone (§16) it is the only toolbar and starts with the canvas switcher (icon, name, the
 * workspace's canvases). */

import { useEffect, useState } from 'react'
import { Link2, LogIn, LogOut, MessageSquarePlus, Minus, Play, Plus, Signpost } from 'lucide-react'
import type { Json } from '../../api/types'
import { pathsOf } from '../../presentation/model'
import { guestLabel, readGuestName, writeGuestName } from '../../state/guest'
import { useOutlineVersion, useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import type { Camera } from '../canvas/render/camera'
import { MAX_ZOOM, MIN_ZOOM } from '../canvas/render/camera'
import { SurfaceSwitcher } from './MainToolbar'
import { MenuList, PopoverPanel, usePopover, type MenuItem } from './popover'
import { useShell, ZOOM_PRESETS } from './shellContext'
import { ObjectImage } from '../../../../presentation/ObjectIcon'
import { useCurrentUser } from '../../../../presentation/currentUser'

export function ViewToolbar({ camera, hasSelection, onFitAll, onFitSelection, annotate }: {
  /** Free Surfaces only: flow pages have no camera, so no zoom. */
  camera: Camera | null
  hasSelection: boolean
  onFitAll: () => void
  onFitSelection: () => void
  /** Absent where there is nothing to annotate (a workspace without canvases). `note`: a free canvas also takes notes
   * from here (a note is an annotation), so the button opens a choice. */
  annotate: { reason: string | null; active: boolean; run: () => void; note?: { reason: string | null; run: () => void } | null } | null
}) {
  const shell = useShell()
  const { t } = useAiwsText()
  const compact = shell.size === 'narrow' || shell.phone
  return (
    <div className="aiws-panel aiws-view-toolbar" role="toolbar" aria-label={shell.phone ? t('aiworkspace.shell.view.canvasTools') : t('aiworkspace.shell.view.tools')} data-testid="aiws-view-toolbar">
      {shell.phone && <SurfaceSwitcher phone />}
      <GuideButton compact={compact} />
      {!shell.phone && <ShowButton />}
      {annotate && <AnnotateButton annotate={annotate} compact={compact} />}
      {camera && <ZoomControl camera={camera} hasSelection={hasSelection} onFitAll={onFitAll} onFitSelection={onFitSelection} />}
      <IdentityButton />
      <ShareButton />
    </div>
  )
}

/** Add an annotation; on a free canvas a menu first offers the other kind of annotation, a note. While a target is
 * being picked the button cancels the pick. */
function AnnotateButton({ annotate, compact }: { annotate: NonNullable<Parameters<typeof ViewToolbar>[0]['annotate']>; compact: boolean }) {
  const { t } = useAiwsText()
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const note = annotate.note
  const menu = Boolean(note) && !annotate.active
  const items: MenuItem[] = note ? [
    { id: 'annotate-object', testId: 'aiws-annotate-object', label: t('aiworkspace.shell.view.annotateObject'), disabled: annotate.reason !== null, reason: annotate.reason, run: annotate.run },
    { id: 'annotate-note', testId: 'aiws-annotate-note', label: t('aiworkspace.shell.view.addNote'), disabled: note.reason !== null, reason: note.reason, explain: true, run: note.run },
  ] : []
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className={`aiws-tool${compact ? '' : ' aiws-tool-text'}`} aria-pressed={annotate.active} disabled={annotate.reason !== null && note?.reason !== null}
        data-testid="aiws-annotate-start" aria-haspopup={menu ? 'menu' : undefined} aria-expanded={menu ? open : undefined}
        title={annotate.active ? t('aiworkspace.shell.view.annotatePick') : menu ? t('aiworkspace.shell.view.annotateOrNote') : annotate.reason ?? t('aiworkspace.shell.view.annotateHint')} aria-label={t('aiworkspace.shell.view.annotate')}
        onClick={menu ? toggle : annotate.run}>
        <MessageSquarePlus size={18} />{!compact && <span>{t('aiworkspace.shell.side.annotations')}</span>}
      </button>
      {open && menu && <MenuList items={items} label={t('aiworkspace.shell.view.annotate')} className="aiws-popover aiws-popover-end" onDone={close} />}
    </span>
  )
}

function useZoom(camera: Camera): number {
  const [zoom, setZoom] = useState(camera.zoom)
  useEffect(() => camera.onChange(() => setZoom(camera.zoom)), [camera])
  return zoom
}

export function ZoomControl({ camera, hasSelection, onFitAll, onFitSelection }: { camera: Camera; hasSelection: boolean; onFitAll: () => void; onFitSelection: () => void }) {
  const { t } = useAiwsText()
  const zoom = useZoom(camera)
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const [custom, setCustom] = useState('')
  const percent = Math.round(zoom * 100)
  const apply = () => {
    const value = Number(custom.replace('%', ''))
    if (Number.isFinite(value) && value > 0) camera.zoomTo(value / 100)
    setCustom('')
  }
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-tool aiws-tool-text aiws-zoom-button" aria-haspopup="dialog" aria-expanded={open} aria-label={t('aiworkspace.shell.view.zoomLevel', { percent })} title={t('aiworkspace.shell.view.zoomMenu')} data-testid="aiws-zoom-menu" onClick={toggle}>
        <span data-testid="aiws-zoom">{percent}%</span>
      </button>
      {open && (
        <PopoverPanel align="end" label={t('aiworkspace.shell.view.zoomMenu')} testId="aiws-zoom-panel" className="aiws-zoom-panel">
          <div className="aiws-zoom-row">
            <button type="button" className="aiws-tool" aria-label={t('aiworkspace.shell.view.zoomOut')} title={t('aiworkspace.shell.view.zoomOut')} data-testid="aiws-zoom-out" disabled={zoom <= MIN_ZOOM + 1e-6} onClick={() => camera.zoomTo(camera.zoom / 1.25)}><Minus size={16} /></button>
            <span className="aiws-zoom-value">{percent}%</span>
            <button type="button" className="aiws-tool" aria-label={t('aiworkspace.shell.view.zoomIn')} title={t('aiworkspace.shell.view.zoomIn')} data-testid="aiws-zoom-in" disabled={zoom >= MAX_ZOOM - 1e-6} onClick={() => camera.zoomTo(camera.zoom * 1.25)}><Plus size={16} /></button>
          </div>
          <div className="aiws-zoom-presets">
            {ZOOM_PRESETS.map((value) => <button key={value} type="button" aria-pressed={percent === value} data-testid={`aiws-zoom-${value}`} onClick={() => camera.zoomTo(value / 100)}>{value}%</button>)}
          </div>
          <form className="aiws-zoom-custom" onSubmit={(event) => { event.preventDefault(); apply() }}>
            <input aria-label={t('aiworkspace.shell.view.customZoom')} placeholder={`${Math.round(MIN_ZOOM * 100)}–${Math.round(MAX_ZOOM * 100)}%`} value={custom} inputMode="numeric" onChange={(event) => setCustom(event.target.value)} data-testid="aiws-zoom-input" />
            <button type="submit">{t('aiworkspace.common.apply')}</button>
          </form>
          <button type="button" className="aiws-popover-item" data-testid="aiws-fit-all" onClick={() => { onFitAll(); close() }}>{t('aiworkspace.shell.fitAll')}</button>
          <button type="button" className="aiws-popover-item" data-testid="aiws-fit-selection" disabled={!hasSelection} title={hasSelection ? undefined : t('aiworkspace.shell.noSelection')} onClick={() => { onFitSelection(); close() }}>{t('aiworkspace.shell.fitSelection')}</button>
        </PopoverPanel>
      )}
    </span>
  )
}

/** "使用引导" (§7.3): one guide starts at once (where the user left it), several are offered in a list. */
function GuideButton({ compact }: { compact: boolean }) {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  useOutlineVersion()
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const guides = pathsOf(store.outline, 'guide')
  if (guides.length === 0) return null
  const begin = (pathId: string) => {
    const saved = store.userState.get<Json>(`guide:${pathId}`) as { index?: number; done?: boolean } | undefined
    shell.setGuide({ pathId, index: saved && !saved.done ? saved.index ?? 0 : 0 })
  }
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className={`aiws-tool${compact ? '' : ' aiws-tool-text'}`} aria-label={t('aiworkspace.shell.guide')} title={t('aiworkspace.shell.view.guideHint')} data-testid="aiws-guide-start"
        aria-pressed={shell.guide !== null} onClick={() => (guides.length === 1 ? begin(guides[0].entity_id) : toggle())}>
        <Signpost size={18} />{!compact && <span>{t('aiworkspace.shell.view.guideShort')}</span>}
      </button>
      {open && (
        <PopoverPanel align="end" label={t('aiworkspace.shell.guide')} testId="aiws-guide-list">
          {guides.map((g) => <button key={g.entity_id} type="button" className="aiws-popover-item" onClick={() => { close(); begin(g.entity_id) }}>{g.title ?? g.name ?? g.entity_id}</button>)}
        </PopoverPanel>
      )}
    </span>
  )
}

/** "开始放映" (§7.2): shown when the workspace has a presentation path. */
function ShowButton() {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  useOutlineVersion()
  if (pathsOf(store.outline, 'presentation').length === 0) return null
  return (
    <button type="button" className="aiws-tool" aria-label={t('aiworkspace.shell.startShow')} title={t('aiworkspace.shell.startShowMore')} data-testid="aiws-show-start" onClick={() => shell.startShow()}>
      <Play size={18} />
    </button>
  )
}

function initial(name: string | null): string {
  if (!name) return '?'
  const trimmed = name.replace(/^did:[^:]+:/, '')
  return (trimmed[0] ?? '?').toUpperCase()
}

function IdentityButton() {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const principal = store.session.principal
  const guest = shell.identity.guest
  const me = useCurrentUser()
  const [guestName, setGuestName] = useState(readGuestName)
  // A Zone account's own picture; a development token identity has none.
  const picture = !shell.identity.dev && !guest && me && me.userId === principal ? me.avatarUrl : undefined
  const shown = guest ? guestName || guestLabel(principal ?? '') : me && me.userId === principal ? me.displayName : principal ?? t('aiworkspace.shell.identity.unknown')
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-avatar" aria-haspopup="dialog" aria-expanded={open} aria-label={t('aiworkspace.shell.identity.label', { name: guest ? t('aiworkspace.shell.identity.guest') : principal ?? t('aiworkspace.common.unknown') })} title={guest ? t('aiworkspace.shell.identity.guestTitle') : principal ?? t('aiworkspace.shell.identity.current')}
        data-testid="aiws-principal" data-principal={principal ?? ''} data-guest={guest ? 'true' : undefined} onClick={toggle}>
        <ObjectImage icons={[picture]} className="aiws-avatar-img" fallback={guest ? t('aiworkspace.shell.identity.guestInitial') : initial(principal)} />
      </button>
      {open && (
        <PopoverPanel align="end" label={t('aiworkspace.shell.identity.panel')} testId="aiws-identity-panel" className="aiws-identity-panel">
          <div className="aiws-panel-title">{t('aiworkspace.shell.identity.mine')}</div>
          <div className="aiws-identity-row"><span className="aiws-avatar is-static" aria-hidden="true"><ObjectImage icons={[picture]} className="aiws-avatar-img" fallback={guest ? t('aiworkspace.shell.identity.guestInitial') : initial(principal)} /></span><b data-testid="aiws-identity-name">{shown}</b></div>
          {guest ? (
            <>
              <div className="aiws-muted">{t('aiworkspace.shell.identity.guestNote')}</div>
              <label className="aiws-guest-name">{t('aiworkspace.shell.identity.guestName')}
                <input value={guestName} maxLength={32} placeholder={guestLabel(principal ?? '')} aria-label={t('aiworkspace.shell.identity.guestName')} data-testid="aiws-guest-name"
                  onChange={(event) => { setGuestName(event.target.value); writeGuestName(event.target.value) }} />
              </label>
              <button type="button" className="aiws-popover-item" data-testid="aiws-login" onClick={() => { close(); shell.login() }}>
                <LogIn size={16} /> {t('aiworkspace.shell.loginBuckyos')}
              </button>
            </>
          ) : (
            <>
              <div className="aiws-muted">{shell.identity.dev ? t('aiworkspace.shell.identity.devNote') : t('aiworkspace.shell.identity.zoneAccount')}</div>
              <div className="aiws-panel-title">{t('aiworkspace.shell.identity.online')}</div>
              <div className="aiws-muted" data-testid="aiws-presence-unavailable">{t('aiworkspace.shell.identity.presenceUnavailable')}</div>
              <button type="button" className="aiws-popover-item" data-testid="aiws-logout" disabled={shell.identity.dev} title={shell.identity.dev ? t('aiworkspace.shell.identity.devNoSession') : undefined}
                onClick={() => { close(); shell.logout() }}>
                <LogOut size={16} /> {t('aiworkspace.shell.identity.logout')}
              </button>
            </>
          )}
        </PopoverPanel>
      )}
    </span>
  )
}


function ShareButton() {
  const shell = useShell()
  const { t } = useAiwsText()
  return (
    <button type="button" className={`aiws-tool${shell.size === 'narrow' || shell.phone ? '' : ' aiws-tool-text'} is-primary`} aria-label={t('aiworkspace.shell.share.label')} title={t('aiworkspace.shell.share.label')} aria-haspopup="dialog" data-testid="aiws-share" onClick={() => shell.openDialog({ kind: 'share' })}>
      <Link2 size={18} />{shell.size !== 'narrow' && !shell.phone && <span>{t('aiworkspace.shell.share.button')}</span>}
    </button>
  )
}
