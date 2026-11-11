import { useContext, useState } from 'react'
import { Play, Square, Undo2 } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { useWindowDialog } from '../../../../desktop/windows/dialogs'
import { getAgentPanelSource } from '../../agentpanel/source'
import { confirmGroupAction } from '../../confirmDialog'
import type { MessageObject } from '../../protocol/msgobj'
import { ConversationMessageActionsContext, type ConversationMessageActions } from '../history/actions'
import { turnMode, type ForegroundMode } from '../input/foregroundState'
import { taskWorklogTarget } from '../worklog/model'
import { useWorkCards } from '../worksessions'
import { flattenWork, isWorkFinished } from '../worksessions/model'
import { useTaskSummary } from './index'

type TurnAction = 'stop' | 'retract' | 'resume'

const buttonClass = 'mh-turn-action'

/**
 * What the owner may do to the Turn in front of the conversation, from the
 * line above the composer (the placeholder itself carries no controls): stop
 * it (the session and its context stay, the work it already started keeps
 * running), stop it and retract the input, or — after the host restarted in
 * the middle of it, or while it is paused — resume it. Nothing here undoes
 * what the Agent already did. Rendered only for the owner of the Agent
 * (`agentTurn` of the conversation's actions) and only while the Turn is
 * running, interrupted or paused.
 */
export function TurnControls(props: { anchor: MessageObject; taskId: string }) {
  const { agentTurn } = useContext(ConversationMessageActionsContext)
  // Conversations without the owner's controls (another's Agent, an embedded conversation view) render nothing and need no dialog host.
  return agentTurn ? <OwnerTurnControls {...props} agentTurn={agentTurn} /> : null
}

function OwnerTurnControls({ anchor, taskId, agentTurn }: { anchor: MessageObject; taskId: string; agentTurn: NonNullable<ConversationMessageActions['agentTurn']> }) {
  const { t } = useI18n()
  const dialog = useWindowDialog()
  const { snapshot, children } = useTaskSummary(taskId)
  const task = snapshot?.status === 'ready' ? snapshot.task : undefined
  const mode: ForegroundMode = turnMode(task)
  const target = task ? taskWorklogTarget(task) : null
  // The work the Turn started is only looked at while there is something to ask about it (a finished Turn reads nothing here).
  const request = target && mode !== 'idle' && (children?.items.length ?? 0) > 0 ? { agentDid: target.agentDid, sessionId: target.sessionId, taskId } : undefined
  const cards = useWorkCards(request, false)
  const works = flattenWork(cards?.cards?.sessions ?? []).filter(card => !isWorkFinished(card.state)).length
  const [acting, setActing] = useState<{ action: TurnAction; mode: ForegroundMode; revision: number } | null>(null)
  const [failed, setFailed] = useState(false)
  if (!target || mode === 'idle') return null
  // The request shows as "stopping…" / "resuming…" until the Turn's task says something new.
  const pending = acting && acting.mode === mode && acting.revision === task?.revision ? acting.action : null
  const input = agentTurn.inputOf(anchor, task)
  const run = async (action: TurnAction) => {
    if (pending) return
    const worksNote = works > 0 ? ` ${t('messagehub.turn.worksKeepRunning', undefined, { count: works })}` : ''
    if (action === 'retract' && !await confirmGroupAction(dialog, t, { title: t('messagehub.turn.retract'), body: `${t('messagehub.turn.retractConfirm')}${worksNote}`, confirm: t('messagehub.turn.retract') })) return
    if (action === 'stop' && works > 0 && !await confirmGroupAction(dialog, t, { title: t('messagehub.turn.stop'), body: `${t('messagehub.turn.stopConfirm')}${worksNote}`, confirm: t('messagehub.turn.stop') })) return
    setFailed(false)
    setActing({ action, mode, revision: task?.revision ?? 0 })
    try {
      if (action === 'retract') await agentTurn.retract(input!)
      else if (action === 'resume') await getAgentPanelSource().resume(target.agentDid, target.sessionId)
      else await getAgentPanelSource().stop(target.agentDid, target.sessionId, 'turn')
    } catch {
      setActing(null)
      setFailed(true)
    }
  }
  const stopping = pending === 'stop' || pending === 'retract'
  return (
    <div className="flex flex-wrap items-center gap-1.5" data-testid="turn-controls" data-mode={mode} data-pending={pending ?? undefined}>
      {mode !== 'running' ? <button type="button" className={buttonClass} disabled={!!pending} onClick={() => void run('resume')} data-testid="turn-resume"><Play size={11} aria-hidden />{t(pending === 'resume' ? 'messagehub.turn.resuming' : 'messagehub.turn.resume')}</button> : null}
      <button type="button" className={buttonClass} disabled={!!pending} onClick={() => void run('stop')} title={t('messagehub.turn.stopHint')} data-testid="turn-stop"><Square size={11} aria-hidden />{t(stopping ? 'messagehub.turn.stopping' : 'messagehub.turn.stop')}</button>
      {mode === 'running' && input ? <button type="button" className={buttonClass} disabled={!!pending} onClick={() => void run('retract')} title={t('messagehub.turn.retractHint')} data-testid="turn-retract"><Undo2 size={11} aria-hidden />{t('messagehub.turn.retract')}</button> : null}
      {failed ? <span role="alert" className="text-[12px] text-[color:var(--cp-danger)]">{t('messagehub.turn.failed')}</span> : null}
    </div>
  )
}
