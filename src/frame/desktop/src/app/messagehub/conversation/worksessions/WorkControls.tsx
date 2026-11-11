import { useState } from 'react'
import { Pause, Play, Square } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { useWindowDialog } from '../../../../desktop/windows/dialogs'
import { getAgentPanelSource } from '../../agentpanel/source'
import { confirmGroupAction } from '../../confirmDialog'
import { workControls, type WorkCard } from './model'

type WorkAction = 'pause' | 'resume' | 'stop'

/**
 * Pause / resume / stop of one work session, for the owner of the Agent.
 * Pausing a plan pauses its Todo sessions, resuming brings back what was
 * paused with it, stopping ends the session and everything under it (asked
 * first). `only` narrows what is offered (a bubble only stops).
 */
export function WorkControls({ agentDid, card, title, only, onChanged }: { agentDid: string; card: WorkCard; title: string; only?: readonly WorkAction[]; onChanged: () => void }) {
  const { t } = useI18n()
  const dialog = useWindowDialog()
  const [pending, setPending] = useState<WorkAction | null>(null)
  const [failed, setFailed] = useState(false)
  const can = workControls(card)
  const offered = (['pause', 'resume', 'stop'] as const).filter(action => can[action] && (!only || only.includes(action)))
  if (offered.length === 0) return null
  const run = async (action: WorkAction) => {
    if (pending) return
    if (action === 'stop' && !await confirmGroupAction(dialog, t, { title: t('messagehub.work.control.stop'), body: t('messagehub.work.control.stopConfirm', undefined, { title }), confirm: t('messagehub.work.control.stop') })) return
    setPending(action); setFailed(false)
    try {
      const source = getAgentPanelSource()
      if (action === 'pause') await source.pause(agentDid, card.session_id)
      else if (action === 'resume') await source.resume(agentDid, card.session_id)
      else await source.stop(agentDid, card.session_id, 'session')
      onChanged()
    } catch {
      setFailed(true)
    } finally {
      setPending(null)
    }
  }
  const icons = { pause: Pause, resume: Play, stop: Square }
  return (
    <span className="flex shrink-0 items-center gap-0.5" onClick={event => event.stopPropagation()} onKeyDown={event => event.stopPropagation()} data-testid="work-controls" data-failed={failed || undefined}>
      {offered.map(action => {
        const Icon = icons[action]
        const label = `${t(pending === action ? `messagehub.work.control.${action}Pending` : `messagehub.work.control.${action}`)}: ${title}`
        return <button key={action} type="button" className="mh-work-control" disabled={!!pending} aria-label={label} title={failed ? `${t('messagehub.work.control.failed')} · ${label}` : label} onClick={() => void run(action)} data-testid={`work-${action}`}><Icon size={13} aria-hidden /></button>
      })}
    </span>
  )
}
