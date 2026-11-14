/* Public class cards of the home (UI framework PRD §9.2): a presentation session the caller may open without a link,
 * across Workspaces (`session.list_open`, K7) — their own, the ones they present or joined, the open rooms of courseware
 * they may read. "我主讲" marks the ones they present (Q7). A click goes in (live, not started) or replays (ended,
 * saved), in a tab of its own on the courseware's origin; an expired one says why it cannot be opened. */

import { Presentation, Radio } from 'lucide-react'
import { useAiwsText } from '../useAiwsText'
import { sessionAction } from '../ui/home/model'
import { MY_ROLE_KEY, STATE_KEY } from './labels'
import type { OpenSession } from './types'
import './session.css'

/** `onOpen`: the card was clicked and the session can be opened (the home notes it in "Recent", then opens it). */
export function SessionCard({ session, onOpen }: { session: OpenSession; onOpen: (session: OpenSession) => void }) {
  const { t, tNode, dateTime } = useAiwsText()
  const action = sessionAction(session)
  const from = session.baseline?.source.title
  const closed = action === 'expired' || !session.url
  const live = session.state === 'live'
  const Icon = live ? Radio : Presentation
  return (
    <div className="aiws-home-card aiws-home-session" data-testid={`aiws-my-session-${session.session_id}`} data-session-id={session.session_id} data-state={session.state} data-role={session.my_role} data-action={action}>
      <button type="button" className="aiws-home-card-main" data-testid="aiws-my-session-open" disabled={closed}
        title={action === 'expired' ? t('aiworkspace.home.session.expired') : undefined}
        onClick={() => onOpen(session)}>
        <span className={`aiws-home-cover is-session${live ? ' is-live' : ''}`} aria-hidden="true"><Icon size={22} /></span>
        <span className="aiws-home-card-body">
          <span className="aiws-home-card-title aiws-user-text">{session.title}</span>
          <span className="aiws-home-card-info">
            <span className={`aiws-chip${live ? ' aiws-chip-ok' : ''}`} data-testid="aiws-home-session-state">{t(STATE_KEY[session.state])}</span>
            <span>{tNode('aiworkspace.home.session.presenter', { name: <span className="aiws-user-text">{session.presenter}</span> })}</span>
            <span>{dateTime(session.started_at ?? session.created_at)}</span>
          </span>
          {from && <span className="aiws-home-card-info aiws-muted">{tNode('aiworkspace.session.mine.from', { workspace: <span className="aiws-user-text">{from}</span> })}</span>}
          <span className="aiws-home-card-marks">
            {session.presenting
              ? <span className="aiws-chip aiws-chip-derived" data-testid="aiws-home-presenting">{t(MY_ROLE_KEY.presenter)}</span>
              : session.my_role === 'owner' || session.my_role === 'audience' ? <span className="aiws-chip">{t(MY_ROLE_KEY[session.my_role])}</span> : null}
            {action === 'expired' && <span className="aiws-muted" data-testid="aiws-home-session-expired">{t('aiworkspace.home.session.expired')}</span>}
          </span>
        </span>
        {!closed && <span className="aiws-home-card-go">{action === 'replay' ? t('aiworkspace.session.create.replay') : t('aiworkspace.session.create.open')}</span>}
      </button>
    </div>
  )
}
