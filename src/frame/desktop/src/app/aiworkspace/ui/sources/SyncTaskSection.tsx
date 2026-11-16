/* A data source written by an Agent's sync task (AI Native 余下任务 §8.2, `source.sync`). Everyone sees which task writes
 * it; the owner reads the task from the Agent with their own session (`sync.get`): version, schedule, hosts, whether it
 * runs, its last run and failures; turns it on or off (`sync.set_enabled`) and sets its declared secrets
 * (`sync.set_secret`, write-only: a value is never read back or shown). Running it now is the source's refresh
 * (SourceSection), so the data version follows. */

import { useCallback, useState } from 'react'
import { agentCall } from '../../api/transport'
import { useLoad } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'

export interface SyncRef { agent_id: string; task_id: string }

interface SyncRun { run_id: string; at: string; ended_at?: string; status: 'running' | 'succeeded' | 'failed'; kind?: string; error?: string; records?: number }

interface SyncTask {
  task_id: string
  title?: string
  version?: number | string
  hosts?: string[]
  schedule?: { every_secs?: number } | null
  secrets?: { name: string; set: boolean }[]
  enabled: boolean
  state?: 'ok' | 'failing' | 'disabled'
  last_run?: SyncRun | null
  failures?: number
}

const RUN_TEXT: Record<SyncRun['status'], string> = {
  running: 'aiworkspace.datasource.sync.run.running', succeeded: 'aiworkspace.datasource.sync.run.succeeded', failed: 'aiworkspace.datasource.sync.run.failed',
}

function text(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

/** `version`: reload the task when it changes (a refresh of the source ran it). */
export function SyncTaskSection({ sync, owner, version }: { sync: SyncRef; owner: boolean; version: string }) {
  const { t, dateTime } = useAiwsText()
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)
  const [drafts, setDrafts] = useState<Record<string, string>>({})
  const load = useCallback(async (): Promise<{ task?: SyncTask; error?: string } | null> => {
    if (!owner) return null
    try {
      return { task: await agentCall<SyncTask>(sync.agent_id, 'sync.get', { task_id: sync.task_id }) }
    } catch (error) {
      return { error: text(error) }
    }
  }, [owner, sync.agent_id, sync.task_id])
  const read = useLoad(load, version)
  const task = read.data?.task ?? null
  const act = (method: string, params: Record<string, unknown>, done?: () => void) => {
    setBusy(true)
    setMessage(null)
    agentCall(sync.agent_id, method, { task_id: sync.task_id, ...params })
      .then(() => { done?.(); read.reload() })
      .catch((error: unknown) => setMessage(t('aiworkspace.datasource.sync.failed', { error: text(error) })))
      .finally(() => setBusy(false))
  }
  const run = task?.last_run ?? null
  const every = task?.schedule?.every_secs
  return (
    <div className="aiws-wish-section" data-testid="aiws-sync-task" data-agent={sync.agent_id} data-task={sync.task_id} data-enabled={task ? String(task.enabled) : ''}>
      <div className="aiws-muted">{t('aiworkspace.datasource.sync.by', { task: sync.task_id, agent: sync.agent_id })}</div>
      {read.data?.error && <div className="aiws-error" role="alert" data-testid="aiws-sync-unreachable">{t('aiworkspace.datasource.sync.unreachable', { error: read.data.error })}</div>}
      {task && (
        <>
          <div data-testid="aiws-sync-summary" data-state={task.state ?? ''}>
            {[task.title || task.task_id, task.version !== undefined ? t('aiworkspace.datasource.sync.version', { version: task.version }) : '',
              every ? t('aiworkspace.datasource.sync.every', { minutes: Math.round(every / 60) }) : '',
              t(task.enabled ? (task.state === 'failing' ? 'aiworkspace.datasource.sync.state.failing' : 'aiworkspace.datasource.sync.state.on') : 'aiworkspace.datasource.sync.state.off')].filter(Boolean).join(' · ')}
          </div>
          {task.hosts && task.hosts.length > 0 && <div className="aiws-muted" data-testid="aiws-sync-hosts">{t('aiworkspace.datasource.sync.hosts', { hosts: task.hosts.join(', ') })}</div>}
          {run && (
            <div className={run.status === 'failed' ? 'aiws-error' : 'aiws-muted'} data-testid="aiws-sync-last-run" data-status={run.status}>
              {t(RUN_TEXT[run.status], { at: dateTime(run.at), error: run.error ?? '' })}{typeof run.records === 'number' ? ` · ${t('aiworkspace.datasource.records', { count: run.records })}` : ''}
            </div>
          )}
          {(task.failures ?? 0) > 0 && <div className="aiws-warning" role="status" data-testid="aiws-sync-failures">{t('aiworkspace.datasource.sync.failures', { count: task.failures ?? 0 })}</div>}
          <div className="aiws-inline-form">
            <button type="button" data-testid="aiws-sync-toggle" disabled={busy} onClick={() => act('sync.set_enabled', { enabled: !task.enabled })}>
              {t(task.enabled ? 'aiworkspace.datasource.sync.disable' : 'aiworkspace.datasource.sync.enable')}
            </button>
          </div>
          {(task.secrets ?? []).length > 0 && <div className="aiws-muted">{t('aiworkspace.datasource.sync.secrets')}</div>}
          {(task.secrets ?? []).map((secret) => (
            <div key={secret.name} className="aiws-inline-form" data-testid={`aiws-sync-secret-${secret.name}`} data-set={String(secret.set)}>
              <label>
                <code>{secret.name}</code>
                <span className="aiws-muted">{t(secret.set ? 'aiworkspace.datasource.sync.secret.set' : 'aiworkspace.datasource.sync.secret.unset')}</span>
              </label>
              <input type="password" autoComplete="new-password" data-testid={`aiws-sync-secret-input-${secret.name}`} aria-label={secret.name}
                placeholder={t(secret.set ? 'aiworkspace.datasource.sync.secret.replace' : 'aiworkspace.datasource.sync.secret.enter')}
                value={drafts[secret.name] ?? ''} onChange={(e) => { const value = e.target.value; setDrafts((d) => ({ ...d, [secret.name]: value })) }} />
              <button type="button" data-testid={`aiws-sync-secret-save-${secret.name}`} disabled={busy || !drafts[secret.name]}
                onClick={() => act('sync.set_secret', { name: secret.name, value: drafts[secret.name] }, () => setDrafts((d) => ({ ...d, [secret.name]: '' })))}>
                {t('aiworkspace.datasource.sync.secret.save')}
              </button>
              {secret.set && (
                <button type="button" data-testid={`aiws-sync-secret-clear-${secret.name}`} disabled={busy} onClick={() => act('sync.set_secret', { name: secret.name, value: null })}>
                  {t('aiworkspace.datasource.sync.secret.clear')}
                </button>
              )}
            </div>
          ))}
        </>
      )}
      {message && <div className="aiws-error" role="alert" data-testid="aiws-sync-error">{message}</div>}
    </div>
  )
}
