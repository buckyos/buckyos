import { useState } from 'react'
import { t, type TextKey } from '../i18n'
import { fmtDuration, fmtTime, fmtTokens, useQuery } from '../lib'
import { dataModel, type GoalDefinition, type JobRun, type JobRunStatus, type JobView } from '../model'
import { Badge, ConfirmDialog, Empty, ErrorBox, Mono, Note, Panel, Pre, Tag, Time, type ConfirmRequest } from '../ui'

/** The host keeps the logs of this many runs. */
const RUNS = 20

const RUN_LABEL: Record<JobRunStatus, TextKey> = { ok: 'jobOk', failed: 'jobFailed', timeout: 'jobTimeout', killed: 'jobKilled' }
const RUN_TONE: Record<JobRunStatus, string> = { ok: 'ok', failed: 'bad', timeout: 'bad', killed: 'warn' }
const TRIGGER_LABEL: Record<string, TextKey> = { schedule: 'jobTriggerSchedule', install: 'jobTriggerInstall', manual: 'jobTriggerManual' }
const KIND_LABEL: Record<string, TextKey> = { transient: 'jobKindTransient', auth: 'jobKindAuth', program: 'jobKindProgram', budget: 'jobKindBudget' }
const named = (labels: Record<string, TextKey>, name: string) => (labels[name] ? t(labels[name]) : name)

const took = (ms: number) => (ms < 1000 ? `${ms}ms` : fmtDuration(Math.round(ms / 1000)))

const timetable = (job: JobView) =>
  job.cron ? t('gateCron', { expr: job.cron, zone: job.timezone || 'UTC' }) : job.every_secs ? t('goalEvery', { n: fmtDuration(job.every_secs) }) : ''

/** Running, or switched off, or how its last run ended. */
function JobState({ job }: { job: JobView }) {
  if (job.running) return <Badge value={t('jobRunning')} tone="running" />
  if (!job.enabled) return <Badge value={t('goalSwitchedOff')} tone="finished" />
  if (job.last_run) return <Badge value={t(RUN_LABEL[job.last_run.status])} tone={RUN_TONE[job.last_run.status]} />
  return <Badge value={t('jobNeverRun')} />
}

const runFacts = (run: JobRun): string =>
  [
    took(run.ended_at_ms - run.at_ms),
    named(TRIGGER_LABEL, run.trigger),
    run.kind ? named(KIND_LABEL, run.kind) : '',
    run.exit_code ? t('jobExit', { code: run.exit_code }) : '',
    run.attempts > 1 ? t('jobAttempts', { n: run.attempts }) : '',
    run.tokens ? t('jobTokens', { n: fmtTokens(run.tokens) }) : '',
  ]
    .filter(Boolean)
    .join(' · ')

function JobLog({ goalId, jobId, runId }: { goalId: string; jobId: string; runId: string }) {
  const log = useQuery(['goals.job_log', goalId, jobId, runId], () => dataModel.goalJobLog(goalId, jobId, runId), false)
  return (
    <div className="mt-1" data-testid="job-log">
      <ErrorBox error={log.error} />
      {log.data ? <Pre value={log.data} /> : !log.error && <Empty>{log.data === undefined ? t('loading') : t('jobNoLog')}</Empty>}
    </div>
  )
}

function Runs({ goalId, jobId }: { goalId: string; jobId: string }) {
  const runs = useQuery(['goals.job_runs', goalId, jobId], () => dataModel.goalJobRuns(goalId, jobId, RUNS))
  const [log, setLog] = useState<string | null>(null)
  return (
    <div className="mt-3 rounded-lg border border-line px-3 py-2" data-testid="job-runs">
      <ErrorBox error={runs.error} />
      {runs.data?.length ? (
        <ul className="divide-y divide-line text-xs leading-5">
          {[...runs.data].reverse().map((run) => (
            <li key={run.run_id} className="py-2 first:pt-0 last:pb-0" data-testid="job-run">
              <div className="flex flex-wrap items-baseline gap-x-3">
                <Badge value={t(RUN_LABEL[run.status])} tone={RUN_TONE[run.status]} />
                <Time ms={run.at_ms} />
                <Tag>v{run.version}</Tag>
                <span className="text-mute">{runFacts(run)}</span>
                <button className="text-accent hover:underline" aria-expanded={log === run.run_id} onClick={() => setLog(log === run.run_id ? null : run.run_id)}>
                  {t(log === run.run_id ? 'jobLogHide' : 'jobLogShow')}
                </button>
              </div>
              {run.summary && <div className="break-words">{run.summary}</div>}
              {run.metrics != null && <Mono className="block text-mute">{JSON.stringify(run.metrics)}</Mono>}
              {log === run.run_id && <JobLog goalId={goalId} jobId={jobId} runId={run.run_id} />}
            </li>
          ))}
        </ul>
      ) : (
        !runs.error && <Empty>{runs.data ? t('jobNoRuns') : t('loading')}</Empty>
      )}
    </div>
  )
}

/** `runs`: the goal is active, so its jobs run. */
function JobRow({ job, runs, busy, act, ask }: {
  job: JobView
  runs: boolean
  busy: boolean
  act: (action: () => Promise<string>) => void
  ask: (request: Omit<ConfirmRequest, 'run'> & { act: () => Promise<string> }) => void
}) {
  const [open, setOpen] = useState(false)
  const { goal_id: goalId, job_id: id, title } = job
  const last = job.last_run
  const earlier = job.versions.filter((v) => v !== job.version).reverse()
  return (
    <li className="py-3 text-xs leading-5 first:pt-0 last:pb-0" data-testid={`job-${id}`}>
      <div className="flex flex-wrap items-baseline gap-x-3">
        <span className="min-w-0 break-words font-medium">{title}</span>
        <span data-testid="job-state"><JobState job={job} /></span>
        <Mono className="text-mute">{id} · v{job.version}</Mono>
      </div>
      <div className="text-mute" data-testid="job-timetable">
        {timetable(job)}
        {job.enabled && runs && job.next_at_ms ? ` · ${t('jobNext')} ${fmtTime(job.next_at_ms)}` : ''}
      </div>
      {job.running && (
        <div className="text-mute" data-testid="job-running">
          {t('jobRunning')} · {t('gateSince')} <Time ms={job.running.at_ms} /> · v{job.running.version} · {named(TRIGGER_LABEL, job.running.trigger)}
        </div>
      )}
      {last && (
        <div data-testid="job-last-run">
          <span className="text-mute">
            {t('jobLast')}: {(job.running || !job.enabled) && `${t(RUN_LABEL[last.status])} · `}<Time ms={last.at_ms} /> · {runFacts(last)}
          </span>
          {last.summary && <span className="block break-words">{last.summary}</span>}
        </div>
      )}
      {job.failures > 0 && <Note tone="bad" testId="job-failures" className="leading-5 text-bad">{t('jobFailures', { n: job.failures })}</Note>}
      <div className="mt-2 flex flex-wrap gap-2">
        <button
          className="btn"
          disabled={busy || !runs || !job.enabled || Boolean(job.running)}
          onClick={() => act(() => dataModel.runGoalJob(goalId, id).then(() => t('jobDoneRun', { title })))}
        >
          {t('jobRun')}
        </button>
        {job.enabled ? (
          <button
            className="btn"
            disabled={busy}
            onClick={() =>
              ask({
                title: t('jobDisableTitle', { title }),
                detail: t('jobDisableHint'),
                confirmLabel: t('skDisable'),
                act: () => dataModel.setGoalJobEnabled(goalId, id, false).then(() => t('jobDoneDisabled', { title })),
              })
            }
          >
            {t('skDisable')}
          </button>
        ) : (
          <button className="btn" disabled={busy} onClick={() => act(() => dataModel.setGoalJobEnabled(goalId, id, true).then(() => t('jobDoneEnabled', { title })))}>
            {t('skEnable')}
          </button>
        )}
        {earlier.length > 0 && (
          <select
            className="input"
            aria-label={t('jobRollback')}
            disabled={busy}
            value=""
            onChange={(e) => {
              const v = Number(e.target.value)
              ask({
                title: t('jobRollbackTitle', { title, v }),
                detail: t('jobRollbackHint', { v }),
                confirmLabel: t('jobRollbackConfirm'),
                act: () => dataModel.rollbackGoalJob(goalId, id, v).then((saved) => t('jobDoneRollback', { title, v: saved.version })),
              })
            }}
          >
            <option value="">{t('jobRollback')}</option>
            {earlier.map((v) => <option key={v} value={v}>v{v}</option>)}
          </select>
        )}
        <button
          className="btn text-bad"
          disabled={busy}
          onClick={() =>
            ask({
              title: t('jobDeleteTitle', { title }),
              detail: t('jobDeleteHint'),
              confirmLabel: t('memDelete'),
              danger: true,
              act: () => dataModel.deleteGoalJob(goalId, id).then(() => t('jobDoneDeleted', { title })),
            })
          }
        >
          {t('memDelete')}
        </button>
        <button className="btn" aria-expanded={open} onClick={() => setOpen(!open)} data-testid="job-runs-toggle">
          {t(open ? 'jobRunsHide' : 'jobRunsShow')}
        </button>
      </div>
      {open && <Runs goalId={goalId} jobId={id} />}
    </li>
  )
}

/** The goal's jobs: programs the host runs on a timetable, without a model. `off`: the goal is switched off. */
export function JobsPanel({ goal, off, onDone }: { goal: GoalDefinition; off: boolean; onDone: () => void }) {
  const jobs = useQuery(['goals.jobs', goal.goal_id], () => dataModel.goalJobs(goal.goal_id))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const [done, setDone] = useState('')
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const runs = goal.status === 'active' && !off
  const finished = (what: string) => {
    setDone(what)
    onDone()
  }
  const act = (action: () => Promise<string>) => {
    setBusy(true)
    setError(null)
    setDone('')
    action()
      .then(finished)
      .catch(setError)
      .finally(() => setBusy(false))
  }
  return (
    <Panel title={t('jobsTitle')} count={jobs.data?.length ?? 0} testId="goal-jobs">
      <ErrorBox error={jobs.error ?? error} />
      {jobs.data?.length ? (
        <>
          <p className="mb-3 max-w-3xl text-xs leading-5 text-mute">
            {t('jobsHint')}{!runs && <span className="text-warn" data-testid="jobs-asleep"> {t('jobsAsleep')}</span>}
          </p>
          <ul className="divide-y divide-line">
            {jobs.data.map((job) => (
              <JobRow
                key={job.job_id}
                job={job}
                runs={runs}
                busy={busy}
                act={act}
                ask={({ act: confirmed, ...request }) => setConfirm({ ...request, run: async () => finished(await confirmed()) })}
              />
            ))}
          </ul>
        </>
      ) : (
        !jobs.error && <Empty>{jobs.data ? t('jobsNone') : t('loading')}</Empty>
      )}
      {done && (
        <Note tone="ok" testId="job-result" className="mt-3">
          {t('opDone', { what: done })}
        </Note>
      )}
      {confirm && <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />}
    </Panel>
  )
}
