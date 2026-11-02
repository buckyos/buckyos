/* 我的公开课 on the Workspace list: the sessions this account created, presents or joined, across Workspaces, newest
 * first — live ones to go back into, ended and saved ones to replay. A room opens on its courseware's own origin. */

import { useCallback } from 'react'
import type { Transport } from '../api/transport'
import { openWorkspacePage } from '../launch'
import { useLoad } from '../state/hooks'
import { useAiwsText } from '../useAiwsText'
import { listMySessions } from './api'
import { MY_ROLE_KEY, STATE_KEY } from './labels'
import type { MySession } from './types'
import './session.css'

/** `version`: bumped by the list's refresh. */
export function MySessions({ transport, version }: { transport: Transport; version: number }) {
  const { t, dateTime } = useAiwsText()
  const load = useCallback(() => listMySessions(transport), [transport])
  const sessions = useLoad<MySession[]>(load, version)
  return (
    <section className="aiws-my-sessions" data-testid="aiws-my-sessions">
      <b>{t('aiworkspace.session.mine.title')}</b>
      {sessions.error && <div className="aiws-error" role="alert">{t('aiworkspace.session.mine.loadFailed', { error: sessions.error })}</div>}
      {sessions.data?.length === 0 && <div className="aiws-muted">{t('aiworkspace.session.mine.empty')}</div>}
      {sessions.data && sessions.data.length > 0 && (
        <ul className="aiws-session-list">
          {sessions.data.map((s) => {
            const from = s.baseline?.source.title
            const url = s.state === 'expired' ? undefined : s.url
            return (
              <li key={s.session_id} data-testid={`aiws-my-session-${s.session_id}`} data-state={s.state} data-role={s.my_role}>
                <span className={`aiws-session-dot${s.state === 'live' ? ' is-on' : ''}`} />
                <span className="aiws-grow">{s.title}{from && <span className="aiws-muted"> · {t('aiworkspace.session.mine.from', { workspace: from })}</span>}</span>
                <span className="aiws-chip">{t(MY_ROLE_KEY[s.my_role])}</span>
                <span className="aiws-muted">{t(STATE_KEY[s.state])} · {dateTime(s.started_at ?? s.created_at)}</span>
                {url && (
                  <button type="button" data-testid="aiws-my-session-open" onClick={() => { void openWorkspacePage(url, { newTab: true }) }}>
                    {s.state === 'ended' || s.state === 'archived' ? t('aiworkspace.session.create.replay') : t('aiworkspace.session.create.open')}
                  </button>
                )}
              </li>
            )
          })}
        </ul>
      )}
    </section>
  )
}
