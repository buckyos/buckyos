/* The home's workspace cards (UI framework PRD §9.2): a click anywhere on the card opens the workspace; the rest — a
 * new tab, Clone, export, offline, details, delete — is in its ⋯ menu. The information line: the owner (when it is not
 * me), the last modification, my role; the marks: prepared for offline, public, a public class live now (K6). Covers
 * are not generated yet: an icon on a colour derived from the workspace id, so cards tell apart. */

import type { CSSProperties } from 'react'
import { LayoutDashboard, MoreHorizontal, Plus, WifiOff } from 'lucide-react'
import type { WorkspaceListItem } from '../../api/types'
import type { PreparedHint } from '../../offline/holder'
import { useAiwsText } from '../../useAiwsText'
import { MenuButton, type MenuItem } from '../shell/popover'
import { hueOf, isMine, ROLE_KEY, roleOf } from './model'

export interface WorkspaceCardActions {
  openTab: () => void
  clone: () => void
  exportPackage: () => void
  prepare: () => void
  details: () => void
  remove: () => void
}

function Cover({ id, offline = false }: { id: string; offline?: boolean }) {
  const Icon = offline ? WifiOff : LayoutDashboard
  return <span className="aiws-home-cover" aria-hidden="true" style={{ '--aiws-home-hue': hueOf(id) } as CSSProperties}><Icon size={22} /></span>
}

export function WorkspaceCard({ workspace, me, openedAt, prepared, opening, busy, restricted, offlineReason, onOpen, actions }: {
  workspace: WorkspaceListItem
  /** Who I am (the owner is named when it is someone else). */
  me: string | null
  /** When I last opened it ("Recent" shows it instead of the modification). */
  openedAt?: string
  prepared: boolean
  opening: boolean
  /** A card action is running: the others wait. */
  busy: boolean
  restricted: boolean
  /** Why this browser cannot hold an offline copy, or null. */
  offlineReason: string | null
  onOpen: () => void
  actions: WorkspaceCardActions
}) {
  const { t, dateTime } = useAiwsText()
  const can = (capability: string) => workspace.capabilities.includes(capability as never)
  const role = roleOf(workspace, me)
  const owner = !isMine(workspace, me) && workspace.owner_principal ? workspace.owner_principal : null
  const live = (workspace.live_sessions ?? 0) > 0
  const published = workspace.public === true
  const menu = (): MenuItem[] => [
    { id: 'open-tab', label: t('aiworkspace.home.menu.openTab'), run: actions.openTab, testId: 'aiws-home-open-tab' },
    ...(can('manage') && !restricted ? [{ id: 'clone', label: t('aiworkspace.core.list.clone'), disabled: busy, run: actions.clone, testId: 'aiws-clone' }] : []),
    ...(can('export') ? [{ id: 'export', label: t('aiworkspace.home.menu.export'), run: actions.exportPackage, testId: 'aiws-home-export' }] : []),
    { id: 'prepare', label: t('aiworkspace.shell.prepareOffline'), disabled: busy || offlineReason !== null, reason: offlineReason, explain: true, run: actions.prepare, testId: 'aiws-home-prepare' },
    { id: 'details', label: t('aiworkspace.home.menu.details'), run: actions.details, testId: 'aiws-home-details' },
    ...(can('manage') ? [{ id: 'sep', separator: true }, { id: 'delete', label: t('aiworkspace.core.list.delete'), danger: true, disabled: busy, run: actions.remove, testId: 'aiws-delete' }] : []),
  ]
  return (
    <div className="aiws-home-card" data-testid="aiws-workspace-card" data-workspace-id={workspace.workspace_id} data-role={role} data-opening={opening || undefined}>
      <button type="button" className="aiws-home-card-main" data-testid="aiws-open" disabled={opening} onClick={onOpen}>
        <Cover id={workspace.workspace_id} />
        <span className="aiws-home-card-body">
          <span className="aiws-home-card-title aiws-user-text">{workspace.title}</span>
          <span className="aiws-home-card-info">
            {owner && <span className="aiws-user-text" data-testid="aiws-home-owner">{t('aiworkspace.home.owner', { name: owner })}</span>}
            {openedAt ? <span data-testid="aiws-home-opened">{t('aiworkspace.home.lastOpened', { at: dateTime(openedAt) })}</span>
              : workspace.updated_at && <span data-testid="aiws-home-modified">{t('aiworkspace.home.modified', { at: dateTime(workspace.updated_at) })}</span>}
            <span data-testid="aiws-home-role">{t(ROLE_KEY[role])}</span>
            {opening && <span role="status">{t('aiworkspace.core.list.opening')}</span>}
          </span>
          {(prepared || live || published) && (
            <span className="aiws-home-card-marks">
              {prepared && <span className="aiws-chip" data-testid="aiws-prepared-chip" title={t('aiworkspace.core.list.preparedChipTitle')}>{t('aiworkspace.core.list.preparedChip')}</span>}
              {published && <span className="aiws-chip" data-testid="aiws-home-public-chip" title={t('aiworkspace.home.publicChipTitle')}>{t('aiworkspace.home.publicChip')}</span>}
              {live && <span className="aiws-chip aiws-chip-ok" data-testid="aiws-home-live-chip" title={t('aiworkspace.home.liveChipTitle', { count: workspace.live_sessions ?? 0 })}>{t('aiworkspace.home.liveChip')}</span>}
            </span>
          )}
        </span>
      </button>
      <MenuButton items={menu} floating align="end" className="aiws-home-card-more" label={t('aiworkspace.home.more')} testId="aiws-home-card-more" menuTestId="aiws-home-card-menu">
        <MoreHorizontal size={18} />
      </MenuButton>
    </div>
  )
}

/** The first card of "All workspaces": the New dialog, on its blank or its template tab. */
export function NewWorkspaceCard({ onNew, onTemplate }: { onNew: () => void; onTemplate: () => void }) {
  const { t } = useAiwsText()
  return (
    <div className="aiws-home-card is-new" data-testid="aiws-home-new-card">
      <button type="button" className="aiws-home-card-main" data-testid="aiws-home-new" onClick={onNew}>
        <span className="aiws-home-cover is-new" aria-hidden="true"><Plus size={24} /></span>
        <span className="aiws-home-card-body">
          <span className="aiws-home-card-title">{t('aiworkspace.home.newCard')}</span>
          <span className="aiws-home-card-info">{t('aiworkspace.home.newCardHint')}</span>
        </span>
      </button>
      <button type="button" className="aiws-link aiws-home-new-template" data-testid="aiws-new-template" onClick={onTemplate}>{t('aiworkspace.core.list.newFromTemplate')}</button>
    </div>
  )
}

/** A workspace prepared for offline on this device, when the list cannot be read (the offline fallback). */
export function PreparedCard({ hint, opening, onOpen }: { hint: PreparedHint; opening: boolean; onOpen: () => void }) {
  const { t } = useAiwsText()
  return (
    <div className="aiws-home-card" data-testid="aiws-workspace-card" data-workspace-id={hint.workspace_id} data-offline="true">
      <button type="button" className="aiws-home-card-main" data-testid="aiws-open" disabled={opening} aria-label={t('aiworkspace.core.list.openOffline')} onClick={onOpen}>
        <Cover id={hint.workspace_id} offline />
        <span className="aiws-home-card-body">
          <span className="aiws-home-card-title aiws-user-text">{hint.title}</span>
          <span className="aiws-home-card-info">{opening ? t('aiworkspace.core.list.opening') : t('aiworkspace.core.list.preparedAt', { id: hint.workspace_id, at: hint.prepared_at })}</span>
          <span className="aiws-home-card-marks"><span className="aiws-chip">{t('aiworkspace.core.list.offlineCopy')}</span></span>
        </span>
      </button>
    </div>
  )
}
