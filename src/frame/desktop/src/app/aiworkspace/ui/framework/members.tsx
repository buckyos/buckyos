/* Who is here (UI 框架 PRD §4.2, §4.5; K2): the avatar stack of the collaboration zone, the online count of the status
 * bar and the online members panel both open. The window registers while the Workspace is open (`PresenceKeeper`,
 * mounted by FrameworkHost whatever the view), so the others see it from the start — on a phone too.
 *
 * The stack: me first, the others overlapping to the right, `+N` past 5 / 3 / 1 faces by size class. The panel: one row
 * per person (all their windows and devices together) — face, name (a guest's chosen name), group (only where the
 * service lets the reader see it: everyone's to a manager, one's own to anyone else, Q6), the canvas they are on (a click
 * goes there). My row carries what the identity panel had: the account and signing out, or a guest's name and signing
 * in. A manager finds "成员与权限…" there. Without the service (offline, refused) the list is me alone, said so — never
 * the grant list or the authors of Commits standing in for who is online. */

import { useEffect, useState, type CSSProperties } from 'react'
import { LogIn, LogOut, Users, X } from 'lucide-react'
import type { PresenceMember } from '../../api/types'
import { guestLabel, readGuestName, writeGuestName } from '../../state/guest'
import { useOutlineVersion, useStore } from '../../state/hooks'
import { presenceGroupLabel, presenceName, presenceOf, usePresence } from '../../state/presence'
import { useAiwsText } from '../../useAiwsText'
import { Modal } from '../shell/dialogs'
import { PopoverPanel, usePopover } from '../shell/popover'
import { useShell } from '../shell/shellContext'
import { ObjectImage } from '../../../../presentation/ObjectIcon'
import { useCurrentUser } from '../../../../presentation/currentUser'
import './framework.css'

/** Registers this window with the Workspace's presence while mounted (nothing drawn). */
export function PresenceKeeper() {
  const store = useStore()
  useEffect(() => presenceOf(store).retain(), [store])
  return null
}

function initial(name: string | null): string {
  if (!name) return '?'
  const trimmed = name.replace(/^did:[^:]+:/, '')
  return (trimmed[0] ?? '?').toUpperCase()
}

/** A hue from the principal: faces without a picture still tell apart. */
function hueOf(text: string): number {
  let h = 2166136261
  for (let i = 0; i < text.length; i += 1) { h ^= text.charCodeAt(i); h = Math.imul(h, 16777619) }
  return (h >>> 0) % 360
}

/** How many faces the stack shows in each size class (§4.2): 5, 3, 1. */
const STACK_SIZE = { wide: 5, medium: 3, narrow: 1 } as const

/** The signed-in person's own name and picture (a Zone account's display name; a guest's chosen name). */
function useSelf() {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  const principal = store.session.principal
  const guest = shell.identity.guest
  const me = useCurrentUser()
  const [guestName, setGuestName] = useState(readGuestName)
  // A Zone account's own picture; a development token identity has none.
  const own = !shell.identity.dev && !guest && me && me.userId === principal
  const name = guest ? guestName || guestLabel(principal ?? '') : own ? me.displayName : principal ?? t('aiworkspace.shell.identity.unknown')
  const face = <ObjectImage icons={[own ? me.avatarUrl : undefined]} className="aiws-avatar-img" fallback={guest ? t('aiworkspace.shell.identity.guestInitial') : initial(principal)} />
  return { principal, guest, name, face, guestName, setGuestName }
}

function Face({ member }: { member: PresenceMember }) {
  const { t } = useAiwsText()
  return <>{member.guest ? t('aiworkspace.shell.identity.guestInitial') : initial(presenceName(member))}</>
}

const hueStyle = (principal: string) => ({ '--aiws-avatar-hue': hueOf(principal) } as CSSProperties)

/** The avatar stack of the collaboration zone; a click opens the online members. */
export function AvatarStack() {
  const shell = useShell()
  const { t } = useAiwsText()
  const presence = usePresence()
  const self = useSelf()
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const others = presence.members.filter((m) => !m.self)
  const max = shell.phone ? 1 : STACK_SIZE[shell.size]
  const shownOthers = others.slice(0, Math.max(0, max - 1))
  const hidden = others.length - shownOthers.length
  return (
    <span ref={bindAnchor} className="aiws-anchor aiws-avatar-stack" data-testid="aiws-collab-avatars" data-count={presence.members.length} data-live={presence.live ? 'true' : undefined}>
      <button ref={bindTrigger} type="button" className="aiws-avatar" aria-haspopup="dialog" aria-expanded={open}
        aria-label={t('aiworkspace.shell.identity.label', { name: self.guest ? t('aiworkspace.shell.identity.guest') : self.principal ?? t('aiworkspace.common.unknown') })}
        title={self.guest ? t('aiworkspace.shell.identity.guestTitle') : self.principal ?? t('aiworkspace.shell.identity.current')}
        data-testid="aiws-principal" data-principal={self.principal ?? ''} data-guest={self.guest ? 'true' : undefined} onClick={toggle}>
        {self.face}
      </button>
      {shownOthers.map((m) => (
        <button key={m.principal} type="button" className="aiws-avatar is-other" style={hueStyle(m.principal)} aria-label={presenceName(m)} title={presenceName(m)}
          aria-haspopup="dialog" aria-expanded={open} data-testid="aiws-avatar-other" data-principal={m.principal} onClick={toggle}>
          <Face member={m} />
        </button>
      ))}
      {hidden > 0 && (
        <button type="button" className="aiws-avatar-more" aria-label={t('aiworkspace.framework.members.stack', { count: presence.members.length })} title={t('aiworkspace.framework.members.stack', { count: presence.members.length })}
          data-testid="aiws-avatar-more" onClick={toggle}>+{hidden}</button>
      )}
      {open && (
        <PopoverPanel align="end" label={t('aiworkspace.framework.members.panel')} testId="aiws-identity-panel" className="aiws-identity-panel aiws-members-panel">
          <MembersList onDone={close} />
        </PopoverPanel>
      )}
    </span>
  )
}

/** The online count of the status bar (only while the service answers): it opens the online members — the way in on a
 * phone, which has no collaboration zone (§6.1). */
export function OnlineCount() {
  const shell = useShell()
  const { t } = useAiwsText()
  const presence = usePresence()
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  if (!presence.live) return null
  const label = t('aiworkspace.activity.online', { count: presence.online })
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-panel aiws-status-online" aria-haspopup="dialog" aria-expanded={open} aria-label={t('aiworkspace.members.openList', { label })}
        title={t('aiworkspace.framework.members.panel')} data-testid="aiws-status-online" data-count={presence.online} onClick={toggle}>
        <Users size={14} aria-hidden="true" /><span>{label}</span>
      </button>
      {open && (shell.phone
        ? <MembersSheet onClose={close} />
        : (
          <PopoverPanel align="end" label={t('aiworkspace.framework.members.panel')} testId="aiws-members-popover" className="aiws-popover-up aiws-members-panel">
            <MembersList onDone={close} />
          </PopoverPanel>
        ))}
    </span>
  )
}

/** The panel from the bottom (a phone). */
function MembersSheet({ onClose }: { onClose: () => void }) {
  const { t } = useAiwsText()
  return (
    <Modal label={t('aiworkspace.framework.members.panel')} testId="aiws-members-sheet" className="aiws-members-sheet" onClose={onClose}>
      <div className="aiws-dialog-head">
        <b>{t('aiworkspace.framework.members.panel')}</b><span className="aiws-grow" />
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.common.close')} title={t('aiworkspace.common.close')} data-testid="aiws-members-sheet-close" onClick={onClose}><X size={18} /></button>
      </div>
      <MembersList onDone={onClose} headless />
    </Modal>
  )
}

/** The online members, me first. */
function MembersList({ onDone, headless = false }: { onDone: () => void; headless?: boolean }) {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  useOutlineVersion()
  const presence = usePresence()
  const self = useSelf()
  const manager = store.session.info().capabilities.includes('manage')
  const surfaceTitle = (id: string | null) => {
    const surface = id ? store.outline.get(id) : undefined
    return surface && !surface.deleted ? surface.title ?? surface.name ?? surface.entity_id : null
  }
  const go = (surfaceId: string) => {
    onDone()
    shell.selectSurface(surfaceId)
    if (shell.topMode === 'sources') shell.setTopMode('canvas')
  }
  const where = (member: PresenceMember) => {
    const title = surfaceTitle(member.surface_id)
    if (!title || !member.surface_id) return null
    const surfaceId = member.surface_id
    return (
      <button type="button" className="aiws-link aiws-member-surface" data-testid="aiws-member-surface" data-surface-id={surfaceId}
        title={t('aiworkspace.members.goTo', { name: title })} onClick={() => go(surfaceId)}>{title}</button>
    )
  }
  const meMember = presence.members.find((m) => m.self) ?? null
  const myGroup = meMember ? presenceGroupLabel(meMember) : null
  return (
    <div className="aiws-members" data-testid="aiws-members-list" data-live={presence.live ? 'true' : 'false'} data-online={presence.online}>
      {!headless && <div className="aiws-panel-title">{t('aiworkspace.framework.members.panel')}{presence.live ? ` · ${presence.online}` : ''}</div>}
      <div className="aiws-member-row is-self" data-testid="aiws-member-me" data-principal={self.principal ?? ''}>
        <span className="aiws-avatar is-static" aria-hidden="true">{self.face}</span>
        <span className="aiws-member-main">
          <b data-testid="aiws-identity-name">{self.name}</b>
          <span className="aiws-member-meta">
            {myGroup && <span className="aiws-chip" data-testid="aiws-member-group">{myGroup}</span>}
            {meMember && surfaceTitle(meMember.surface_id) && <span className="aiws-muted aiws-member-here">{surfaceTitle(meMember.surface_id)}</span>}
          </span>
        </span>
        <span className="aiws-chip">{t('aiworkspace.framework.members.me')}</span>
      </div>
      {self.guest ? (
        <>
          <div className="aiws-muted">{t('aiworkspace.shell.identity.guestNote')}</div>
          <label className="aiws-guest-name">{t('aiworkspace.shell.identity.guestName')}
            <input value={self.guestName} maxLength={32} placeholder={guestLabel(self.principal ?? '')} aria-label={t('aiworkspace.shell.identity.guestName')} data-testid="aiws-guest-name"
              onChange={(event) => { self.setGuestName(event.target.value); writeGuestName(event.target.value) }} onBlur={() => { void presenceOf(store).beat() }} />
          </label>
          <button type="button" className="aiws-popover-item" data-testid="aiws-login" onClick={() => { onDone(); shell.login() }}>
            <LogIn size={16} /> {t('aiworkspace.shell.loginBuckyos')}
          </button>
        </>
      ) : (
        <>
          <div className="aiws-muted">{shell.identity.dev ? t('aiworkspace.shell.identity.devNote') : t('aiworkspace.shell.identity.zoneAccount')}</div>
          <button type="button" className="aiws-popover-item" data-testid="aiws-logout" disabled={shell.identity.dev} title={shell.identity.dev ? t('aiworkspace.shell.identity.devNoSession') : undefined}
            onClick={() => { onDone(); shell.logout() }}>
            <LogOut size={16} /> {t('aiworkspace.shell.identity.logout')}
          </button>
        </>
      )}
      {presence.live
        ? presence.members.filter((m) => !m.self).map((m) => {
          const group = presenceGroupLabel(m)
          return (
            <div key={m.principal} className="aiws-member-row" data-testid="aiws-member-row" data-principal={m.principal}>
              <span className="aiws-avatar is-static is-other" style={hueStyle(m.principal)} aria-hidden="true"><Face member={m} /></span>
              <span className="aiws-member-main">
                <b className="aiws-user-text">{presenceName(m)}</b>
                <span className="aiws-member-meta">
                  {group && <span className="aiws-chip" data-testid="aiws-member-group">{group}</span>}
                  {where(m)}
                  {m.devices > 1 && <span className="aiws-muted">{t('aiworkspace.activity.devices', { count: m.devices })}</span>}
                </span>
              </span>
            </div>
          )
        })
        : <div className="aiws-muted" data-testid="aiws-presence-unavailable">{t('aiworkspace.activity.offline.presence')}</div>}
      {presence.live && presence.online === 1 && <div className="aiws-muted" data-testid="aiws-members-alone">{t('aiworkspace.members.alone')}</div>}
      {manager && !self.guest && (
        <button type="button" className="aiws-popover-item" data-testid="aiws-members-permissions" onClick={() => { onDone(); shell.openPermissions() }}>
          <Users size={16} /> {t('aiworkspace.framework.collab.members')}
        </button>
      )}
    </div>
  )
}
