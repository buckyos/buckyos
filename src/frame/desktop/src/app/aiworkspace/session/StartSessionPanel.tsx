/* eslint-disable react-refresh/only-export-components -- the panel and the address it opens */
/* "开公开课" (第四期规划 §3.1, Q03, Q13): a session of a presentation path — a room with its own links, recorded from
 * the start, on the Workspace's own host. The room's policy is set here (guest interaction, voice, the audience it
 * expects); the courseware's earlier sessions are listed below it (§8: 历次演示). */

import { useEffect, useState } from 'react'
import { describeError } from '../api/session'
import type { Transport } from '../api/transport'
import { openWorkspacePage } from '../launch'
import { workspacePageUrl, workspaceSlug } from '../links'
import type { WorkspaceStore } from '../state/store'
import { useAiwsText } from '../useAiwsText'
import { createSession, listSessions } from './api'
import type { SessionInfo } from './types'
import { STATE_KEY } from './labels'

/** A session's address: the Workspace's own page with `?session=`. */
export function sessionPageUrl(store: WorkspaceStore, sessionId: string): string {
  const info = store.session.info()
  const page = workspacePageUrl((info as { public_base?: string | null }).public_base ?? null, info.title) ?? `${window.location.origin}/${workspaceSlug(info.title)}`
  return `${page}?session=${encodeURIComponent(sessionId)}`
}

export function StartSessionPanel({ store, transport, pathId, title, disabled, onStarted }: { store: WorkspaceStore; transport: Transport; pathId: string; title: string; disabled: boolean; onStarted: () => void }) {
  const { t } = useAiwsText()
  const [name, setName] = useState(title)
  const [guests, setGuests] = useState(false)
  const [voice, setVoice] = useState(true)
  const [expected, setExpected] = useState(50)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const go = async () => {
    setBusy(true)
    setError(null)
    try {
      const { session } = await createSession(transport, { workspace_id: store.session.workspaceId, path_id: pathId, title: name.trim() || title, policy: { guest_interact: guests, voice, expected_audience: expected } })
      await openWorkspacePage(sessionPageUrl(store, session.session_id), { newTab: true })
      onStarted()
    } catch (e) {
      setError(describeError(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="aiws-session-create" data-testid="aiws-session-create">
      <div className="aiws-muted">{t('aiworkspace.session.create.help')}</div>
      <label className="aiws-inline-form">{t('aiworkspace.session.create.title')}
        <input data-testid="aiws-session-create-title" value={name} maxLength={256} onChange={(event) => setName(event.target.value)} />
      </label>
      <label className="aiws-check"><input type="checkbox" data-testid="aiws-session-create-guests" checked={guests} onChange={(event) => setGuests(event.target.checked)} />{t('aiworkspace.session.create.guests')}</label>
      <label className="aiws-check"><input type="checkbox" data-testid="aiws-session-create-voice" checked={voice} onChange={(event) => setVoice(event.target.checked)} />{t('aiworkspace.session.create.voice')}</label>
      <label className="aiws-inline-form">{t('aiworkspace.session.create.expected')}
        <input type="number" min={1} max={10000} value={expected} onChange={(event) => setExpected(Math.max(1, Math.min(10000, Number(event.target.value) || 1)))} />
      </label>
      {error && <div className="aiws-error" role="alert">{t('aiworkspace.session.create.failed', { detail: error })}</div>}
      <div className="aiws-dialog-actions">
        <button type="button" className="is-primary" data-testid="aiws-session-create-go" disabled={disabled || busy} onClick={() => { void go() }}>{busy ? t('aiworkspace.session.create.creating') : t('aiworkspace.session.create.go')}</button>
      </div>
      <SessionHistory store={store} transport={transport} />
    </div>
  )
}

/** The sessions of this courseware the caller may see (§8): live ones to rejoin, saved ones to replay. */
export function SessionHistory({ store, transport }: { store: WorkspaceStore; transport: Transport }) {
  const { t, dateTime } = useAiwsText()
  const [sessions, setSessions] = useState<SessionInfo[] | null>(null)
  useEffect(() => {
    let live = true
    listSessions(transport, store.session.workspaceId).then((s) => { if (live) setSessions(s) }, () => { if (live) setSessions([]) })
    return () => { live = false }
  }, [store, transport])
  if (!sessions || sessions.length === 0) return null
  return (
    <div className="aiws-session-history" data-testid="aiws-session-history">
      <b>{t('aiworkspace.session.create.history')}</b>
      <ul className="aiws-session-list">
        {sessions.map((s) => (
          <li key={s.session_id} data-testid={`aiws-session-history-${s.session_id}`}>
            <span className="aiws-grow">{s.title}</span>
            <span className="aiws-muted">{t(STATE_KEY[s.state])} · {dateTime(s.created_at)}</span>
            {s.state !== 'expired' && <button type="button" onClick={() => { void openWorkspacePage(sessionPageUrl(store, s.session_id), { newTab: true }) }}>{s.state === 'archived' || s.state === 'ended' ? t('aiworkspace.session.create.replay') : t('aiworkspace.session.create.open')}</button>}
          </li>
        ))}
      </ul>
    </div>
  )
}
