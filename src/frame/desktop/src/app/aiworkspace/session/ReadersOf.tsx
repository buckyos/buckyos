/* Who may come in without a link (UI 框架 PRD Q19): the readers of the session's permission object — an entity of the
 * courseware (by default the canvas of the path's first step), judged by permission path, kept by id — and the way to
 * choose another: the permission editor's object picker (a bottom sheet on a phone). Shown where a public class is
 * created and where its presenter or owner manages it (the room's links, the replay). */

import { useEffect, useRef, useState } from 'react'
import { resolvePath } from '../api/paths'
import { describeError } from '../api/session'
import { useStore } from '../state/hooks'
import { ObjectPicker } from '../ui/permissions/ResourcePathInput'
import { useAiwsText } from '../useAiwsText'
import type { SessionChannel } from './channel'
import type { ReadersOf } from './types'
import './session.css'

const ROOT_OBJECT = 'root'

/** The row: who may come in, and 更改… for those who may choose (`onPick` gets the chosen entity's id). */
export function ReadersOfRow({ value, onPick, busy = false, problem = null }: { value: ReadersOf; onPick?: (entityId: string) => void; busy?: boolean; problem?: string | null }) {
  const { t } = useAiwsText()
  const store = useStore()
  const [picking, setPicking] = useState(false)
  const who = !value.alive ? t('aiworkspace.session.readers.gone')
    : value.entity_id === ROOT_OBJECT ? t('aiworkspace.session.readers.workspace')
    : t('aiworkspace.session.readers.of', { name: value.title ?? value.entity_id })
  return (
    <div className="aiws-session-readers" data-testid="aiws-session-readers" data-entity-id={value.entity_id} data-alive={value.alive ? 'true' : 'false'}>
      <div className="aiws-session-readers-line">
        <span className="aiws-muted">{t('aiworkspace.session.readers.title')}</span>
        <b className={value.alive ? undefined : 'aiws-warning'} data-testid="aiws-session-readers-name" title={value.path ?? undefined}>{who}</b>
        {onPick && <button type="button" data-testid="aiws-session-readers-change" disabled={busy} onClick={() => setPicking(true)}>{t('aiworkspace.session.readers.change')}</button>}
      </div>
      <div className="aiws-muted aiws-session-readers-help">{t('aiworkspace.session.readers.help')}</div>
      {problem && <div className="aiws-error" role="alert" data-testid="aiws-session-readers-problem">{problem}</div>}
      {picking && onPick && (
        <ObjectPicker onClose={() => setPicking(false)} onPick={(path) => {
          setPicking(false)
          const picked = path === '/' ? ROOT_OBJECT : resolvePath(store.outline, path)?.entity_id
          if (picked) onPick(picked)
        }} />
      )}
    </div>
  )
}

/** In the room (the presenter's links, the replay): the object as the service names it now (the welcome's, then
 * `session.get`'s — the panel may open again after a change), changed through `session.set_access` by the presenter or
 * the owner; others only see it. */
export function SessionReadersOf({ channel, initial, editable }: { channel: SessionChannel; initial: ReadersOf | undefined; editable: boolean }) {
  const [value, setValue] = useState<ReadersOf | undefined>(initial)
  const [busy, setBusy] = useState(false)
  const [problem, setProblem] = useState<string | null>(null)
  const chosen = useRef(false)
  useEffect(() => {
    let alive = true
    channel.call<{ readers_of?: ReadersOf }>('session.get', {}).then((r) => { if (alive && !chosen.current && r.readers_of) setValue(r.readers_of) }, () => undefined)
    return () => { alive = false }
  }, [channel])
  if (!value) return null
  const pick = async (entityId: string) => {
    chosen.current = true
    setBusy(true)
    setProblem(null)
    try {
      const r = await channel.call<{ readers_of: ReadersOf }>('session.set_access', { readers_of: entityId })
      setValue(r.readers_of)
    } catch (e) {
      setProblem(describeError(e))
    } finally {
      setBusy(false)
    }
  }
  return <ReadersOfRow value={value} onPick={editable ? (id) => { void pick(id) } : undefined} busy={busy} problem={problem} />
}
