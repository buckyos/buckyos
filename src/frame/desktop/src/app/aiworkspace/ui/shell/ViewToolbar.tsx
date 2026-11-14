/* The collaboration zone, top right (UI 框架 PRD §4.2): play | macro | comment | zoom | avatars | share link. Play,
 * macro and comment are single icons with their names in the tooltip; zoom (free canvases only) and the share link
 * stay as they were (Q1, Q2). What the zone holds comes from the registered actions and the toolbar configuration
 * (ui/framework), so an action declared for this zone appears here without a change to this file. In a narrow window
 * the avatar stack shows fewer faces and zoom moves into "more". The avatar stack is who is online (K2,
 * ui/framework/members.tsx). A phone has no collaboration zone (§6.1): its title area is ui/shell/MainToolbar's
 * PhoneTitle. */

import { useEffect, useState } from 'react'
import { Ellipsis, Link2, Minus, Plus } from 'lucide-react'
import { useAiwsText } from '../../useAiwsText'
import type { Camera } from '../canvas/render/camera'
import { MAX_ZOOM, MIN_ZOOM } from '../canvas/render/camera'
import { actionRegistry, candidateItem, toolItemOf, type ActionDef } from '../framework/actions'
import { itemApplies, type ResolvedItem, type ResolvedZone } from '../framework/config'
import { useActionContext, useToolbarLayout } from '../framework/hooks'
import { useMacroWatch } from '../framework/macros'
import { AvatarStack } from '../framework/members'
import { ToolItem } from '../framework/ToolItem'
import '../framework/framework.css'
import type { CanvasCommands } from './MainMenu'
import { MenuButton, PopoverPanel, usePopover, type MenuItem } from './popover'
import { useShell, ZOOM_PRESETS } from './shellContext'

export function ViewToolbar({ canvas, zone }: {
  /** The open canvas's commands (none without a canvas). */
  canvas: CanvasCommands | null
  /** The zone as the toolbar configuration resolves it; the built-in one when absent. */
  zone?: ResolvedZone
}) {
  const shell = useShell()
  const { t } = useAiwsText()
  const actions = useActionContext(canvas)
  useMacroWatch()
  const fallback = useToolbarLayout(undefined).collab
  const items = (zone ?? fallback).items.filter((item) => itemApplies(item, actions))
    .flatMap((item) => { const def = actionRegistry.get(item.action); return def ? [{ item, def }] : [] })
  const narrow = shell.size === 'narrow'
  const inMore = (def: ActionDef) => narrow && def.zone?.overflow === 'narrow'
  const more: MenuItem[] = items.filter(({ def }) => inMore(def)).flatMap(({ item, def }) => {
    if (def.visible && !def.visible(actions)) return []
    const candidates = def.candidates?.(actions, item.args)
    return [candidates ? { id: item.key, testId: `aiws-more-${item.key}`, label: item.title ?? def.label(actions), items: candidates.map(candidateItem) }
      : { id: item.key, testId: `aiws-more-${item.key}`, label: item.title ?? def.label(actions), run: () => def.run?.(actions, item.args) }]
  })
  const render = ({ item, def }: { item: ResolvedItem; def: ActionDef }) => {
    switch (def.zone?.control) {
      case 'zoom': {
        const view = canvas?.view
        return view ? <ZoomControl key={item.key} camera={view.camera} hasSelection={view.hasSelection} onFitAll={view.fitAll} onFitSelection={view.fitSelection} /> : null
      }
      case 'avatars': return <AvatarStack key={item.key} />
      case 'share': return <ShareButton key={item.key} />
    }
    const model = toolItemOf(def, actions, item.args, { title: item.title, icon: item.icon })
    if (!model) return null
    return <ToolItem key={item.key} item={model} testId={item.key === def.id ? def.zone?.testId : `aiws-collab-item-${item.key}`} />
  }
  const shown = items.filter(({ def }) => !inMore(def))
  const firstControl = shown.findIndex(({ def }) => def.zone?.control)
  const before = firstControl < 0 ? shown : shown.slice(0, firstControl)
  const after = firstControl < 0 ? [] : shown.slice(firstControl)
  return (
    <div className="aiws-panel aiws-view-toolbar" role="toolbar" aria-label={t('aiworkspace.framework.zone.collab')} data-testid="aiws-view-toolbar">
      {before.map(render)}
      {after.filter(({ def }) => def.zone?.control !== 'share').map(render)}
      {more.length > 0 && (
        <MenuButton label={t('aiworkspace.framework.more')} testId="aiws-collab-more" menuTestId="aiws-collab-more-list" align="end" items={more}>
          <Ellipsis size={18} />
        </MenuButton>
      )}
      {after.filter(({ def }) => def.zone?.control === 'share').map(render)}
    </div>
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

function ShareButton() {
  const shell = useShell()
  const { t } = useAiwsText()
  return (
    <button type="button" className={`aiws-tool${shell.size === 'narrow' ? '' : ' aiws-tool-text'} is-primary`} aria-label={t('aiworkspace.shell.share.label')} title={t('aiworkspace.shell.share.label')} aria-haspopup="dialog" data-testid="aiws-share" onClick={() => shell.openDialog({ kind: 'share' })}>
      <Link2 size={18} />{shell.size !== 'narrow' && <span>{t('aiworkspace.shell.share.button')}</span>}
    </button>
  )
}
