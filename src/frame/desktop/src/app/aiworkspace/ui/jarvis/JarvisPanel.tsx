/* The Agent's panel (C2-1; PRD §8.3; Q12): a conversation with the viewer's own Agent beside the work — at the side of
 * a wide window, pulled up from the bottom on a phone, the canvas staying usable either way — that hands the Agent the
 * workspace, where the person is and what is selected (context.ts) with the next message they send; the chip above
 * the conversation says what goes along, and the same place is not sent twice. The conversation is the system
 * component on MessageHub's data (SessionConversation), as in the Agent Browser; its session is session.ts's.
 * "协作 ▸ 与 Agent 对话" and the collaboration zone open it; FrameworkHost mounts the host. */

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { Bot, ChevronDown, Paperclip, X } from 'lucide-react'
import { LoadingOrb } from '../../../../components/LoadingOrb'
import type { OutgoingPayload } from '../../../../components/conversation/model/outgoing'
import { SessionConversation } from '../../../../components/conversation/SessionConversation'
import { useOutlineVersion, useStore } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { useAiwsText } from '../../useAiwsText'
import { dataTabsOf } from '../datamode/tabs'
import { useShell } from '../shell/shellContext'
import { panelContext, type PanelContext } from './context'
import { jarvisPanel, openJarvis } from './open'
import { panelAgent, panelSession, type PanelAgent } from './session'
import './jarvis.css'

/** Per workspace, the context last sent or put aside (the chip shows again once the place or the selection moves). */
const settled = new Map<string, string>()

/** A selection on the canvas is the canvas's state: it is read again after each press and key. */
function subscribeInput(notify: () => void): () => void {
  let timer = 0
  const later = () => { window.clearTimeout(timer); timer = window.setTimeout(notify, 0) }
  document.addEventListener('pointerup', later, true)
  document.addEventListener('keyup', later, true)
  return () => { window.clearTimeout(timer); document.removeEventListener('pointerup', later, true); document.removeEventListener('keyup', later, true) }
}

function useContextNow(store: WorkspaceStore): PanelContext {
  const shell = useShell()
  useOutlineVersion()
  const tabs = dataTabsOf(store)
  useSyncExternalStore(tabs.subscribe, tabs.snapshot)
  useSyncExternalStore(subscribeInput, () => store.canvasFocus.current.join('\n'))
  return panelContext(store, shell)
}

type Phase = { kind: 'loading' } | { kind: 'none' } | { kind: 'failed' } | { kind: 'ready'; agent: PanelAgent; sessionId: string }

function usePanelConversation(): { phase: Phase; retry: () => void } {
  const [attempt, setAttempt] = useState(0)
  const [phase, setPhase] = useState<Phase>({ kind: 'loading' })
  useEffect(() => {
    let alive = true
    panelAgent().then(async (agent): Promise<Phase> => (agent ? { kind: 'ready', agent, sessionId: await panelSession(agent) } : { kind: 'none' })).then(
      (next) => { if (alive) setPhase(next) },
      () => { if (alive) setPhase({ kind: 'failed' }) },
    )
    return () => { alive = false }
  }, [attempt])
  return { phase, retry: () => { setPhase({ kind: 'loading' }); setAttempt((n) => n + 1) } }
}

function Status({ testId, title, detail, loading, retry }: { testId: string; title: string; detail?: string; loading?: boolean; retry?: () => void }) {
  const { t } = useAiwsText()
  return (
    <div className="aiws-jarvis-status" role={loading ? 'status' : undefined} data-testid={testId}>
      {loading && <LoadingOrb size={32} />}
      <p className={loading ? undefined : 'aiws-jarvis-status-title'}>{title}</p>
      {detail && <p>{detail}</p>}
      {retry && <button type="button" data-testid={`${testId}-retry`} onClick={retry}>{t('aiworkspace.common.retry')}</button>}
    </div>
  )
}

function JarvisPanel({ onClose }: { onClose: () => void }) {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  const { phase, retry } = usePanelConversation()
  const ctx = useContextNow(store)
  const ws = store.session.workspaceId
  const [settledKey, setSettledKey] = useState(() => settled.get(ws) ?? null)
  const settle = useCallback((key: string) => { settled.set(ws, key); setSettledKey(key) }, [ws])
  const pending = ctx.key !== settledKey
  const latest = useRef({ ctx, pending })
  useEffect(() => { latest.current = { ctx, pending } })
  // a payload's text is decided once: its pending bubble and its send carry the same context
  const decided = useRef(new Map<string, OutgoingPayload>())
  const outgoing = useCallback((payload: OutgoingPayload): OutgoingPayload => {
    if (payload.relatesTo || payload.machine || payload.voice) return payload
    const known = payload.localId ? decided.current.get(payload.localId) : undefined
    if (known) return known
    const now = latest.current
    const next = now.pending ? { ...payload, content: payload.content.trim() ? `${payload.content}\n\n${now.ctx.text}` : now.ctx.text } : payload
    if (now.pending) { settle(now.ctx.key); latest.current = { ...now, pending: false } }
    if (payload.localId) decided.current.set(payload.localId, next)
    return next
  }, [settle])
  const name = phase.kind === 'ready' ? phase.agent.name || t('aiworkspace.jarvis.yourAgent') : t('aiworkspace.jarvis.yourAgent')
  const title = t('aiworkspace.jarvis.title', { name })
  return (
    <section className="aiws-jarvis" data-layout={shell.phone ? 'sheet' : 'side'} aria-label={title} data-testid="aiws-jarvis" data-session={phase.kind === 'ready' ? phase.sessionId : undefined}>
      <header className="aiws-jarvis-head">
        <Bot size={16} aria-hidden="true" />
        <h2>{title}</h2>
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.jarvis.close')} title={t('aiworkspace.jarvis.close')} data-testid="aiws-jarvis-close" onClick={onClose}>
          {shell.phone ? <ChevronDown size={20} /> : <X size={18} />}
        </button>
      </header>
      {pending && phase.kind === 'ready' && (
        <div className="aiws-jarvis-context" role="note" title={t('aiworkspace.jarvis.context.hint')} data-testid="aiws-jarvis-context">
          <Paperclip size={14} aria-hidden="true" />
          <span><b>{t('aiworkspace.jarvis.context.label')}</b> {ctx.summary}</span>
          <button type="button" className="aiws-tool" aria-label={t('aiworkspace.jarvis.context.dismiss')} title={t('aiworkspace.jarvis.context.dismiss')} data-testid="aiws-jarvis-context-dismiss" onClick={() => settle(ctx.key)}><X size={14} /></button>
        </div>
      )}
      <div className="aiws-jarvis-body aiws-system-host">
        {phase.kind === 'ready' ? (
          <SessionConversation
            key={phase.sessionId}
            sessionId={phase.sessionId}
            testId="aiws-jarvis-conversation"
            placeholder={t('aiworkspace.jarvis.placeholder', { name })}
            emptyLabel={t('aiworkspace.jarvis.empty', { name })}
            readOnlyLabel={t('aiworkspace.jarvis.readOnly')}
            outgoing={outgoing}
          />
        ) : phase.kind === 'none' ? <Status testId="aiws-jarvis-none" title={t('aiworkspace.jarvis.none')} detail={t('aiworkspace.jarvis.noneHint')} />
          : phase.kind === 'failed' ? <Status testId="aiws-jarvis-failed" title={t('aiworkspace.jarvis.failed')} retry={retry} />
            : <Status testId="aiws-jarvis-loading" loading title={t('aiworkspace.jarvis.loading')} />}
      </div>
    </section>
  )
}

/** Mounted once by the workspace shell (FrameworkHost): the panel while it is open; not over a show. */
export function JarvisHost() {
  const shell = useShell()
  const open = useSyncExternalStore(jarvisPanel.emitter.subscribe, () => jarvisPanel.open)
  useEffect(() => () => openJarvis(false), [])
  if (!open || shell.topMode === 'show') return null
  return <JarvisPanel onClose={() => openJarvis(false)} />
}
