/* The maintenance Goal of the personal site: what it is for, its jobs (the scripts that sync each website: when each
 * last ran and runs next) and its recent changes, with the Goal itself one press away — the system's Goal
 * conversation, where the owner reads what it did and says something to it. */

import { ChevronRight, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import { GoalConversation } from '../../../components/conversation/goals/GoalConversation'
import { GoalStatusLabel } from '../../../components/conversation/goals/GoalStatusLabel'
import type { Goal } from '../../../components/conversation/goals/model'
import { getGoalSource } from '../../../components/conversation/goals/source'
import { AgentWorklog } from '../../../components/conversation/worklog/AgentWorklog'
import type { WorklogDestination } from '../../../components/conversation/worklog/model'
import { useI18n } from '../../../i18n/provider'
import { formatMoment } from '../reflow/useReflow'
import { useAgentBrowser, useAgentBrowserStore } from '../store/context'
import { maintainedBy } from './controller'
import { readChanges, type GoalChange, type Job } from './manage'
import { MaintainerNote } from './ManagePanel'
import { useSite } from './useSite'

const CHANGES = 12
const VIA = ['owner', 'ui_session', 'work_session', 'goal_session', 'opendan']

type Translate = ReturnType<typeof useI18n>['t']

function jobLine(t: Translate, locale: string, job: Job): string {
  if (job.running) return t('agentBrowser.manage.job.running', 'Running now')
  if (!job.enabled) return t('agentBrowser.manage.job.off', 'Off')
  return job.next_at_ms ? t('agentBrowser.manage.job.next', 'Next run {{time}}', { time: formatMoment(job.next_at_ms, locale) }) : t('agentBrowser.manage.job.unscheduled', 'Not scheduled')
}

export function GoalTab() {
  const store = useAgentBrowserStore()
  const { t, locale } = useI18n()
  const agent = useAgentBrowser((snapshot) => snapshot.agent)
  const talks = useAgentBrowser((snapshot) => snapshot.conversations === 'ready')
  const site = useSite((snapshot) => snapshot.site)
  const jobs = useSite((snapshot) => snapshot.jobs)
  const goalId = site?.maintainer?.goal_id ?? ''
  const mine = maintainedBy(site, agent) === 'this'
  const [goal, setGoal] = useState<Goal | null | undefined>(undefined)
  const [changes, setChanges] = useState<GoalChange[] | null>(null)
  const [conversation, setConversation] = useState(false)
  const [worklog, setWorklog] = useState<WorklogDestination | null>(null)
  useEffect(() => {
    if (!agent || !mine) return
    let alive = true
    getGoalSource().get(agent.agentDid, goalId).then((found) => { if (alive) setGoal(found) }, () => { if (alive) setGoal(null) })
    readChanges(agent, goalId, CHANGES).then((list) => { if (alive) setChanges(list) }, () => { if (alive) setChanges([]) })
    return () => { alive = false }
  }, [agent, mine, goalId])
  if (!agent || !mine) return <div className="ab-manage-body" data-testid="ab-manage-goal"><MaintainerNote /></div>
  const viewerDid = store.conversationContext()?.context.viewerDid
  if (conversation && viewerDid) {
    return (
      <div className="ab-manage-goal" data-testid="ab-manage-goal-conversation">
        <div className="ab-manage-layer" hidden={worklog !== null}>
          <GoalConversation agentDid={agent.agentDid} agentName={agent.name} goalId={goalId} title={goal?.title ?? ''} viewerDid={viewerDid} active={worklog === null} onBack={() => setConversation(false)} onOpenWorklog={setWorklog} />
        </div>
        {worklog ? (
          <section className="ab-manage-layer ab-manage-worklog" data-testid="ab-manage-worklog">
            <header className="ab-chat-sheet-head">
              <h2>{worklog.title}</h2>
              <button type="button" className="ab-icon-button" aria-label={t('common.close')} onClick={() => setWorklog(null)}><X size={18} aria-hidden /></button>
            </header>
            <AgentWorklog initial={worklog} onExit={() => setWorklog(null)} />
          </section>
        ) : null}
      </div>
    )
  }
  return (
    <div className="ab-manage-body" data-testid="ab-manage-goal" data-goal={goalId}>
      {goal === null ? <p className="ab-sheet-empty" data-testid="ab-goal-missing">{t('agentBrowser.manage.goal.missing', '{{name}} has no such Goal any more. Ask it in the conversation to set the maintenance up again.', { name: agent.name })}</p> : (
        <button type="button" className="ab-list-item ab-goal-card" disabled={!goal || !talks || !viewerDid} data-testid="ab-goal-open" onClick={() => setConversation(true)}>
          <span className="ab-list-text">
            <span className="ab-list-title">{goal?.title ?? t('agentBrowser.manage.goal.loading', 'Reading the Goal…')}</span>
            {goal ? <span className="ab-list-detail ab-goal-status"><GoalStatusLabel goal={goal} switchedOff={false} /></span> : null}
            {goal ? <span className="ab-list-detail ab-goal-objective">{goal.objective}</span> : null}
            <span className="ab-list-detail">{t('agentBrowser.manage.goal.hint', 'Open it to read what {{name}} did for the site, and to tell it what should change.', { name: agent.name })}</span>
          </span>
          <ChevronRight size={18} aria-hidden />
        </button>
      )}
      <h3 className="ab-list-heading">{t('agentBrowser.manage.jobs.title', 'Jobs')}</h3>
      {jobs === null ? <p className="ab-sheet-empty" data-testid="ab-jobs-loading">{t('agentBrowser.manage.jobs.loading', 'Reading the jobs…')}</p>
        : jobs.length === 0 ? <p className="ab-sheet-empty" data-testid="ab-jobs-empty">{t('agentBrowser.manage.jobs.empty', 'This Goal has no job yet.')}</p> : (
          <ul className="ab-list ab-manage-rows" data-testid="ab-job-list">
            {jobs.map((job) => (
              <li key={job.job_id} className="ab-manage-row" data-tone={job.running ? 'busy' : !job.enabled ? 'idle' : job.last_run && job.last_run.status !== 'ok' ? 'failed' : 'ok'} data-testid="ab-job" data-job={job.job_id} data-enabled={job.enabled ? 'true' : 'false'}>
                <span className="ab-site-dot" aria-hidden />
                <span className="ab-list-text">
                  <span className="ab-list-title">{job.title}</span>
                  <span className="ab-list-detail">
                    {job.last_run
                      ? t('agentBrowser.manage.job.last', 'Last run {{time}}: {{status}}', { time: formatMoment(job.last_run.ended_at_ms, locale), status: t(`agentBrowser.manage.run.${job.last_run.status}`, job.last_run.status) })
                      : t('agentBrowser.manage.job.never', 'Has not run yet')}
                    {' · '}{jobLine(t, locale, job)}
                  </span>
                  {job.last_run?.summary ? <span className="ab-list-detail ab-manage-wrap">{job.last_run.summary}</span> : null}
                </span>
              </li>
            ))}
          </ul>
        )}
      <h3 className="ab-list-heading">{t('agentBrowser.manage.changes.title', 'Recent changes')}</h3>
      {changes === null ? <p className="ab-sheet-empty">{t('agentBrowser.manage.changes.loading', 'Reading the changes…')}</p>
        : changes.length === 0 ? <p className="ab-sheet-empty" data-testid="ab-changes-empty">{t('agentBrowser.manage.changes.empty', 'Nothing has changed yet.')}</p> : (
          <ul className="ab-list ab-manage-rows" data-testid="ab-change-list">
            {changes.map((change) => (
              <li key={change.seq} className="ab-manage-row" data-testid="ab-change" data-kind={change.kind}>
                <span className="ab-list-text">
                  <span className="ab-list-title">{t(`agentBrowser.manage.change.${change.kind}`, change.kind)}{change.detail ? `: ${change.detail}` : ''}</span>
                  <span className="ab-list-detail">{formatMoment(change.at_ms, locale)}{VIA.includes(change.via) ? ` · ${t(`agentBrowser.manage.via.${change.via}`, change.via, { name: agent.name })}` : ''}</span>
                  {change.reason ? <span className="ab-list-detail ab-manage-wrap">{change.reason}</span> : null}
                </span>
              </li>
            ))}
          </ul>
        )}
    </div>
  )
}
