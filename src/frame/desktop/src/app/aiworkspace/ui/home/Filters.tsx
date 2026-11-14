/* The home's filter settings (UI framework PRD §9.3): workspaces by ownership, my permission and offline copies, with
 * their sort (last modified, last opened, name); public classes by status and "only those I present". A popover under
 * its button on a computer, a panel from the bottom on a phone (§9.4). They are the person's preference (prefs.ts). */

import { useEffect } from 'react'
import { SlidersHorizontal, X } from 'lucide-react'
import { useAiwsText } from '../../useAiwsText'
import { PopoverPanel, useOverlayMounted, usePopover } from '../shell/popover'
import { clearedFilters, filtersActive, type HomeCategory, type HomePrefs, type OwnerFilter, type PermissionFilter, type SessionStateFilter, type WorkspaceSort } from './prefs'

function FilterFields({ category, prefs, onChange }: { category: HomeCategory; prefs: HomePrefs; onChange: (prefs: HomePrefs) => void }) {
  const { t } = useAiwsText()
  const w = prefs.workspaces
  const s = prefs.sessions
  const setW = (patch: Partial<HomePrefs['workspaces']>) => onChange({ ...prefs, workspaces: { ...w, ...patch } })
  const setS = (patch: Partial<HomePrefs['sessions']>) => onChange({ ...prefs, sessions: { ...s, ...patch } })
  return (
    <div className="aiws-home-filters" data-testid="aiws-home-filters">
      {category !== 'sessions' && (
        <fieldset className="aiws-field">
          <legend>{t('aiworkspace.home.filter.workspaces')}</legend>
          <label className="aiws-home-filter-row">{t('aiworkspace.home.filter.owner')}
            <select data-testid="aiws-home-filter-owner" value={w.owner} onChange={(event) => setW({ owner: event.target.value as OwnerFilter })}>
              <option value="all">{t('aiworkspace.home.filter.owner.all')}</option>
              <option value="mine">{t('aiworkspace.home.filter.owner.mine')}</option>
              <option value="shared">{t('aiworkspace.home.filter.owner.shared')}</option>
            </select>
          </label>
          <label className="aiws-home-filter-row">{t('aiworkspace.home.filter.permission')}
            <select data-testid="aiws-home-filter-permission" value={w.permission} onChange={(event) => setW({ permission: event.target.value as PermissionFilter })}>
              <option value="all">{t('aiworkspace.home.filter.permission.all')}</option>
              <option value="edit">{t('aiworkspace.home.filter.permission.edit')}</option>
              <option value="read">{t('aiworkspace.home.filter.permission.read')}</option>
            </select>
          </label>
          <label className="aiws-home-check"><input type="checkbox" data-testid="aiws-home-filter-offline" checked={w.offline} onChange={(event) => setW({ offline: event.target.checked })} /> {t('aiworkspace.home.filter.offline')}</label>
          {category === 'all' ? (
            <label className="aiws-home-filter-row">{t('aiworkspace.home.filter.sort')}
              <select data-testid="aiws-home-filter-sort" value={w.sort} onChange={(event) => setW({ sort: event.target.value as WorkspaceSort })}>
                <option value="modified">{t('aiworkspace.home.filter.sort.modified')}</option>
                <option value="opened">{t('aiworkspace.home.filter.sort.opened')}</option>
                <option value="name">{t('aiworkspace.home.filter.sort.name')}</option>
              </select>
            </label>
          ) : <small className="aiws-muted">{t('aiworkspace.home.filter.recentOrder')}</small>}
        </fieldset>
      )}
      {category !== 'all' && (
        <fieldset className="aiws-field">
          <legend>{t('aiworkspace.home.filter.sessions')}</legend>
          <label className="aiws-home-filter-row">{t('aiworkspace.home.filter.state')}
            <select data-testid="aiws-home-filter-state" value={s.state} onChange={(event) => setS({ state: event.target.value as SessionStateFilter })}>
              <option value="all">{t('aiworkspace.home.filter.state.all')}</option>
              <option value="live">{t('aiworkspace.home.filter.state.live')}</option>
              <option value="upcoming">{t('aiworkspace.home.filter.state.upcoming')}</option>
              <option value="replay">{t('aiworkspace.home.filter.state.replay')}</option>
              <option value="expired">{t('aiworkspace.home.filter.state.expired')}</option>
            </select>
          </label>
          <label className="aiws-home-check"><input type="checkbox" data-testid="aiws-home-filter-mine" checked={s.mine} onChange={(event) => setS({ mine: event.target.checked })} /> {t('aiworkspace.home.filter.mine')}</label>
        </fieldset>
      )}
      <div className="aiws-dialog-actions">
        <button type="button" data-testid="aiws-home-filter-reset" onClick={() => onChange(clearedFilters(prefs))}>{t('aiworkspace.home.filter.reset')}</button>
      </div>
      <small className="aiws-muted">{t('aiworkspace.home.filter.kept')}</small>
    </div>
  )
}

/** The filter button and its popover (a computer). */
export function FilterButton({ category, prefs, onChange }: { category: HomeCategory; prefs: HomePrefs; onChange: (prefs: HomePrefs) => void }) {
  const { t } = useAiwsText()
  const { open, toggle, bindAnchor, bindTrigger } = usePopover()
  const active = filtersActive(category, prefs)
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-home-icon" data-testid="aiws-home-filter" data-active={active || undefined} aria-expanded={open} aria-label={t('aiworkspace.home.filter')} title={t('aiworkspace.home.filter')} onClick={toggle}>
        <SlidersHorizontal size={18} />
      </button>
      {open && <PopoverPanel align="end" label={t('aiworkspace.home.filter')} testId="aiws-home-filter-panel" className="aiws-home-filter-popover"><FilterFields category={category} prefs={prefs} onChange={onChange} /></PopoverPanel>}
    </span>
  )
}

/** The same settings in a panel from the bottom (a phone). */
export function FilterSheet({ category, prefs, onChange, onClose }: { category: HomeCategory; prefs: HomePrefs; onChange: (prefs: HomePrefs) => void; onClose: () => void }) {
  const { t } = useAiwsText()
  useOverlayMounted()
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); onClose() } }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [onClose])
  return (
    <div className="aiws-home-sheet-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
      <div className="aiws-home-sheet" role="dialog" aria-modal="true" aria-label={t('aiworkspace.home.filter')} data-testid="aiws-home-filter-sheet">
        <div className="aiws-home-sheet-grip" aria-hidden="true" />
        <div className="aiws-dialog-head"><b>{t('aiworkspace.home.filter')}</b><span className="aiws-grow" /><button type="button" className="aiws-home-icon" aria-label={t('aiworkspace.common.close')} data-testid="aiws-home-filter-close" onClick={onClose}><X size={18} /></button></div>
        <FilterFields category={category} prefs={prefs} onChange={onChange} />
      </div>
    </div>
  )
}
