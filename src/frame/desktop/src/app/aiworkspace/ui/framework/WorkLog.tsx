/* "最近工作日志" of the status panel (UI 框架 PRD §4.5; K3): what happened here, newest first, as the service reads it
 * for this person — Commits (people, Agents and macros marked apart), wish and macro runs (start, end), imports, shows
 * and public classes, and permission changes for managers. 50 entries, older ones on request; 全部 / 我的 / Agent 与宏.
 * A click locates the entry: the Block on its canvas, else the object in data mode. What is still under way in this
 * window and has no entry yet — uploads into a directory (§7.4) — is listed first, with its progress. */

import { useSyncExternalStore } from 'react'
import { X } from 'lucide-react'
import type { WorkLogFilter } from '../../api/types'
import { useStore, useWorkspaceUi } from '../../state/hooks'
import { describeWorkLogEntry, useWorkLog, workLogActor, workLogLocation } from '../../state/worklog'
import { useAiwsText } from '../../useAiwsText'
import { uploadQueue } from '../directory/uploads'
import { useShell } from '../shell/shellContext'
import { locateWorkLogEntry } from './activity'
import './framework.css'

const FILTERS: { id: WorkLogFilter; label: string }[] = [
  { id: 'all', label: 'aiworkspace.worklog.filter.all' },
  { id: 'mine', label: 'aiworkspace.worklog.filter.mine' },
  { id: 'automation', label: 'aiworkspace.worklog.filter.automation' },
]

export function WorkLogSection() {
  const store = useStore()
  const shell = useShell()
  const ui = useWorkspaceUi()
  const { t, time, dateTime } = useAiwsText()
  const [filter, setFilterState] = useFilter()
  const log = useWorkLog({ filter })
  const today = new Date().toDateString()
  const when = (at: string) => (new Date(at).toDateString() === today ? time(at) : dateTime(at))
  return (
    <section className="aiws-worklog" data-testid="aiws-status-log" data-filter={filter}>
      <div className="aiws-worklog-head">
        <div className="aiws-panel-title">{t('aiworkspace.worklog.title')}</div>
        <span className="aiws-grow" />
        <div className="aiws-segmented" role="group" aria-label={t('aiworkspace.worklog.filter')}>
          {FILTERS.map((f) => (
            <button key={f.id} type="button" aria-pressed={filter === f.id} data-testid={`aiws-status-log-filter-${f.id}`} onClick={() => setFilterState(f.id)}>{t(f.label)}</button>
          ))}
        </div>
      </div>
      <Uploads />
      {log.error && <div className="aiws-muted" role="status" data-testid="aiws-status-log-error">{t('aiworkspace.worklog.failed', { error: log.error })}</div>}
      {log.loading && log.entries.length === 0 && <div className="aiws-muted" role="status">{t('aiworkspace.worklog.loading')}</div>}
      {!log.loading && !log.error && log.entries.length === 0 && <div className="aiws-muted" data-testid="aiws-status-log-empty">{t('aiworkspace.worklog.empty')}</div>}
      {log.entries.length > 0 && (
        <ol className="aiws-worklog-list" data-testid="aiws-status-log-list">
          {log.entries.map((entry) => {
            const target = entry.target && !entry.target.deleted ? entry.target.title?.trim() || entry.target.name || null : null
            const where = workLogLocation(entry)
            const what = describeWorkLogEntry(entry)
            return (
              <li key={entry.id}>
                <button type="button" className="aiws-worklog-entry" data-testid="aiws-status-log-entry" data-id={entry.id} data-kind={entry.kind} data-phase={entry.phase} data-actor-kind={entry.actor_kind}
                  data-target={entry.target?.entity_id} disabled={!where} title={where ? t('aiworkspace.worklog.locate') : undefined}
                  onClick={() => { locateWorkLogEntry(entry, store, shell, ui.openEntity) }}>
                  <span className="aiws-worklog-time">{when(entry.at)}</span>
                  <span className="aiws-worklog-body">
                    <span className="aiws-worklog-actor aiws-user-text" data-actor-kind={entry.actor_kind}>{workLogActor(entry)}</span>
                    <span className="aiws-worklog-what aiws-user-text" data-testid="aiws-status-log-text">{what}</span>
                    {target && !what.includes(target) && <span className="aiws-worklog-target aiws-user-text">{target}</span>}
                  </span>
                </button>
              </li>
            )
          })}
        </ol>
      )}
      {log.more && (
        <button type="button" className="aiws-link aiws-worklog-more" data-testid="aiws-status-log-more" disabled={log.loadingMore} onClick={log.loadMore}>
          {log.loadingMore ? t('aiworkspace.worklog.loading') : t('aiworkspace.worklog.more')}
        </button>
      )}
    </section>
  )
}

/** The filter is this window's: the panel opens on 全部. */
let lastFilter: WorkLogFilter = 'all'
function useFilter(): [WorkLogFilter, (filter: WorkLogFilter) => void] {
  const value = useSyncExternalStore(filterEvents.subscribe, () => lastFilter)
  return [value, (filter) => { lastFilter = filter; filterEvents.emit() }]
}
const filterEvents = (() => {
  const listeners = new Set<() => void>()
  return { subscribe: (l: () => void) => { listeners.add(l); return () => { listeners.delete(l) } }, emit: () => { for (const l of [...listeners]) l() } }
})()

/** Uploads into directories still running in this window, with their progress (§7.4). */
function Uploads() {
  const store = useStore()
  const { t } = useAiwsText()
  const queue = uploadQueue(store)
  useSyncExternalStore(queue.subscribe, queue.snapshot)
  const active = queue.active()
  if (active.length === 0) return null
  return (
    <ul className="aiws-worklog-list is-running" data-testid="aiws-status-log-uploads">
      {active.map((job) => {
        const percent = job.total > 0 ? Math.min(100, Math.round((job.sent / job.total) * 100)) : 0
        return (
          <li key={job.id} className="aiws-worklog-entry is-running" data-testid="aiws-status-log-upload" data-state={job.state}>
            <span className="aiws-worklog-time">{t(job.state === 'waiting' ? 'aiworkspace.worklog.upload.waiting' : job.state === 'saving' ? 'aiworkspace.worklog.upload.saving' : 'aiworkspace.worklog.upload.sending')}</span>
            <span className="aiws-worklog-body">
              <span className="aiws-worklog-what aiws-user-text">{t('aiworkspace.worklog.upload.name', { name: job.name })}</span>
              <progress max={100} value={percent} aria-label={t('aiworkspace.worklog.upload.progress', { name: job.name, percent })} />
            </span>
            <button type="button" className="aiws-tool" aria-label={t('aiworkspace.worklog.upload.cancel', { name: job.name })} title={t('aiworkspace.common.cancel')} data-testid="aiws-status-log-upload-cancel" onClick={() => queue.cancel(job.id)}><X size={14} /></button>
          </li>
        )
      })}
    </ul>
  )
}
