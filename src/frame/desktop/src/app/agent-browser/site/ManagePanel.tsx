/* "Manage" of the personal site, over home: the websites with what each is doing and what the owner can do about it
 * (pause, sync now, remove), the maintenance Goal with its jobs and recent changes, and what is recorded about the
 * owner's reading. Pausing and syncing go to the Goal's jobs with the owner's session; only removing a website is
 * asked of the Agent. */

import { ArrowLeft, BookOpen, Globe, Pause, Play, Plus, RotateCw, Trash2 } from 'lucide-react'
import { useCallback, useState } from 'react'
import { useMobileBackHandler } from '../../../desktop/windows/MobileNavContext'
import { useI18n } from '../../../i18n/provider'
import { formatMoment } from '../reflow/useReflow'
import { Sheet } from '../shell/Sheet'
import { useAgentBrowser, useAgentBrowserStore } from '../store/context'
import { jobOf, maintainedBy, type ManageTab, type SourceAction } from './controller'
import { displayState, type SiteSource } from './data'
import { GoalTab } from './GoalTab'
import { RecordsTab } from './RecordsTab'
import { SOURCE_TONES, sourceText } from './status'
import { useSite, useSiteController } from './useSite'

const TABS: ManageTab[] = ['sources', 'goal', 'records']

/** Why the owner cannot pause, sync or remove websites from here: nobody maintains the site yet, or another Agent does. */
export function MaintainerNote() {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const agent = useAgentBrowser((snapshot) => snapshot.agent)
  const agents = useAgentBrowser((snapshot) => snapshot.agents)
  const site = useSite((snapshot) => snapshot.site)
  const who = maintainedBy(site, agent)
  if (who === 'this' || !agent) return null
  const other = agents.find((item) => item.agentDid === site?.maintainer?.agent)
  return (
    <div className="ab-manage-note" role="note" data-testid="ab-manage-maintainer" data-maintainer={who}>
      <p>
        {who === 'none'
          ? t('agentBrowser.manage.noMaintainer', 'No Agent maintains a reading site for you yet. Once {{name}} has added a website, it is managed here.', { name: agent.name })
          : t('agentBrowser.manage.otherMaintainer', 'This site is maintained by {{maintainer}}. Its websites are paused, synced and removed in that Agent’s browser.', { maintainer: other?.name ?? site?.maintainer?.agent ?? '' })}
      </p>
      {other ? <button type="button" className="ab-button" data-testid="ab-manage-switch" onClick={() => store.selectAgent(other.agentId)}>{t('agentBrowser.site.switchAgent', 'Switch')}</button> : null}
    </div>
  )
}

/** One website: what it is doing, and what the owner can do about it. */
function SourceSheet({ source, onClose }: { source: SiteSource; onClose: () => void }) {
  const controller = useSiteController()
  const store = useAgentBrowserStore()
  const { t, locale } = useI18n()
  const agent = useAgentBrowser((snapshot) => snapshot.agent)
  const site = useSite((snapshot) => snapshot.site)
  const feed = useSite((snapshot) => snapshot.feeds[source.id] ?? null)
  const job = useSite((snapshot) => snapshot.jobs?.find((item) => item.job_id === jobOf(source)) ?? null)
  const acting = useSite((snapshot) => snapshot.acting[source.id] ?? null)
  const removing = useSite((snapshot) => snapshot.removing.includes(source.id))
  const [confirming, setConfirming] = useState(false)
  if (!site || !agent) return null
  const name = agent.name
  const state = displayState(site, source, feed)
  const mine = maintainedBy(site, agent) === 'this'
  if (confirming) {
    return (
      <Sheet
        testId="ab-source-remove-confirm"
        title={t('agentBrowser.manage.remove.title', 'Remove {{site}}?', { site: source.title })}
        onClose={() => setConfirming(false)}
        footer={<>
          <button type="button" className="ab-button" data-testid="ab-source-remove-cancel" onClick={() => setConfirming(false)}>{t('common.cancel')}</button>
          <button type="button" className="ab-button ab-button-danger" data-testid="ab-source-remove-accept" onClick={() => { onClose(); void controller.removeSource(source.id) }}>{t('agentBrowser.manage.remove.accept', 'Remove')}</button>
        </>}
      >
        <p className="ab-dialog-message">{t('agentBrowser.manage.remove.body', '{{name}} is asked in the conversation to take this website out of your reading: it stops being collected. Articles you already have stay readable until they expire.', { name })}</p>
      </Sheet>
    )
  }
  // A website that could not be added has no job to run: trying again is the Agent's.
  const added = source.state !== 'failed' && source.state !== 'preparing'
  const actions: Array<{ id: SourceAction | 'read' | 'original' | 'retry'; icon: React.ReactNode; label: string; run: () => void; off?: boolean; danger?: boolean }> = [
    { id: 'read', icon: <BookOpen size={18} aria-hidden />, label: t('agentBrowser.manage.source.read', 'Read it'), run: () => { controller.closeManage(); controller.go(`s=${source.id}`) } },
    ...(added && source.state !== 'paused' ? [{ id: 'sync' as const, icon: <RotateCw size={18} aria-hidden />, label: state === 'needs_login' ? t('agentBrowser.manage.source.signedIn', 'I have signed in — try again') : state === 'failed' ? t('agentBrowser.manage.source.retry', 'Try again') : t('agentBrowser.manage.source.sync', 'Sync now'), run: () => void controller.syncSource(source.id), off: !mine }] : []),
    ...(source.state === 'failed' ? [{ id: 'retry' as const, icon: <RotateCw size={18} aria-hidden />, label: t('agentBrowser.manage.source.addAgain', 'Ask {{name}} to try adding it again', { name }), run: () => { onClose(); void controller.retrySource(source.id) }, off: !mine }] : []),
    ...(source.state === 'active' ? [{ id: 'pause' as const, icon: <Pause size={18} aria-hidden />, label: t('agentBrowser.manage.source.pause', 'Pause'), run: () => void controller.pauseSource(source.id, true), off: !mine }] : []),
    ...(source.state === 'paused' ? [{ id: 'resume' as const, icon: <Play size={18} aria-hidden />, label: t('agentBrowser.manage.source.resume', 'Resume'), run: () => void controller.pauseSource(source.id, false), off: !mine }] : []),
    ...(source.home_url ? [{ id: 'original' as const, icon: <Globe size={18} aria-hidden />, label: t('agentBrowser.manage.source.original', 'Open the original site'), run: () => { controller.closeManage(); store.openInPage(source.home_url) } }] : []),
    { id: 'remove', icon: <Trash2 size={18} aria-hidden />, label: removing ? t('agentBrowser.manage.source.removing', '{{name}} was asked to remove it', { name }) : t('agentBrowser.manage.source.remove', 'Remove from my reading…'), run: () => setConfirming(true), off: !mine || removing, danger: true },
  ]
  return (
    <Sheet testId="ab-source-sheet" title={source.title} onClose={onClose}>
      <dl className="ab-manage-facts" data-testid="ab-source-facts" data-state={state}>
        <dt>{t('agentBrowser.manage.source.state', 'Now')}</dt><dd>{sourceText(t, locale, state, source, feed)}</dd>
        {job?.last_run ? <><dt>{t('agentBrowser.manage.source.lastRun', 'Last run')}</dt><dd data-testid="ab-source-last-run">{t(`agentBrowser.manage.run.${job.last_run.status}`, job.last_run.status)} · {formatMoment(job.last_run.ended_at_ms, locale)}{job.last_run.summary ? ` · ${job.last_run.summary}` : ''}</dd></> : null}
        {job ? <><dt>{t('agentBrowser.manage.source.next', 'Next sync')}</dt><dd data-testid="ab-source-next">{job.running ? t('agentBrowser.manage.job.running', 'Running now') : !job.enabled ? t('agentBrowser.manage.job.off', 'Off') : job.next_at_ms ? formatMoment(job.next_at_ms, locale) : t('agentBrowser.manage.job.unscheduled', 'Not scheduled')}</dd></> : null}
      </dl>
      {mine ? null : <MaintainerNote />}
      <ul className="ab-list" data-testid="ab-source-actions">
        {actions.map((action) => (
          <li key={action.id}>
            <button type="button" className="ab-list-item" disabled={action.off || acting !== null} data-danger={action.danger ? 'true' : undefined} data-testid={`ab-source-${action.id}`} data-acting={acting === action.id ? 'true' : undefined} onClick={action.run}>
              {action.icon}
              <span className="ab-list-text"><span className="ab-list-title">{action.label}</span></span>
            </button>
          </li>
        ))}
      </ul>
    </Sheet>
  )
}

function SourcesTab() {
  const controller = useSiteController()
  const { t, locale } = useI18n()
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  const site = useSite((snapshot) => snapshot.site)
  const feeds = useSite((snapshot) => snapshot.feeds)
  const jobs = useSite((snapshot) => snapshot.jobs)
  const acting = useSite((snapshot) => snapshot.acting)
  const synced = useSite((snapshot) => snapshot.synced)
  const removing = useSite((snapshot) => snapshot.removing)
  const [opened, setOpened] = useState<string | null>(null)
  const source = site?.sources.find((item) => item.id === opened)
  const add = <button type="button" className="ab-button" data-testid="ab-manage-add" onClick={() => controller.openSheet({ kind: 'start' })}><Plus size={16} aria-hidden />{t('agentBrowser.site.start.sheetTitle', 'Add a website')}</button>
  if (!site || site.sources.length === 0) {
    return (
      <div className="ab-manage-body" data-testid="ab-manage-sources">
        <p className="ab-sheet-empty" data-testid="ab-manage-sources-empty">{t('agentBrowser.manage.sources.empty', 'Your reading has no website yet.')}</p>
        <div className="ab-manage-foot">{add}</div>
      </div>
    )
  }
  return (
    <div className="ab-manage-body" data-testid="ab-manage-sources">
      <MaintainerNote />
      <ul className="ab-list" data-testid="ab-source-list">
        {site.sources.map((item) => {
          // What a website that was added is doing is in its feed: until that was read, the row says nothing about it.
          const read = item.id in feeds || item.state !== 'active'
          const state = displayState(site, item, feeds[item.id] ?? null)
          const running = jobs?.find((job) => job.job_id === jobOf(item))?.running === true
          const doing = removing.includes(item.id) ? t('agentBrowser.manage.source.removing', '{{name}} was asked to remove it', { name })
            : running || acting[item.id] === 'sync' ? t('agentBrowser.manage.source.syncing', 'Syncing…')
            : item.id in synced ? t('agentBrowser.manage.source.syncAsked', 'A sync was asked for')
            : ''
          return (
            <li key={item.id}>
              <button type="button" className="ab-list-item ab-source-row" aria-haspopup="dialog" data-tone={read ? SOURCE_TONES[state] : undefined} data-testid="ab-source" data-source={item.id} data-state={read ? state : undefined} data-doing={doing ? 'true' : undefined} onClick={() => setOpened(item.id)}>
                <span className="ab-site-dot" aria-hidden />
                <span className="ab-list-text">
                  <span className="ab-list-title">{item.title}</span>
                  <span className="ab-list-detail">{read ? sourceText(t, locale, state, item, feeds[item.id] ?? null) : '…'}</span>
                  {doing ? <span className="ab-list-detail ab-source-doing">{doing}</span> : null}
                </span>
              </button>
            </li>
          )
        })}
      </ul>
      <div className="ab-manage-foot">{add}</div>
      {source ? <SourceSheet key={source.id} source={source} onClose={() => setOpened(null)} /> : null}
    </div>
  )
}

/** The pages of "manage"; the phone's Back leaves it. */
export function ManagePanel({ tab }: { tab: ManageTab }) {
  const controller = useSiteController()
  const { t } = useI18n()
  const close = useCallback(() => controller.closeManage(), [controller])
  useMobileBackHandler(close)
  const labels: Record<ManageTab, string> = {
    sources: t('agentBrowser.manage.tab.sources', 'Websites'),
    goal: t('agentBrowser.manage.tab.goal', 'Maintenance'),
    records: t('agentBrowser.manage.tab.records', 'Records'),
  }
  return (
    <section className="ab-manage" aria-label={t('agentBrowser.manage.title', 'Manage my reading')} data-testid="ab-manage" data-tab={tab}>
      <header className="ab-manage-head">
        <button type="button" className="ab-icon-button" aria-label={t('agentBrowser.manage.back', 'Back to reading')} data-testid="ab-manage-back" onClick={close}><ArrowLeft size={18} aria-hidden /></button>
        <div className="ab-modes ab-manage-tabs" role="tablist" aria-label={t('agentBrowser.manage.title', 'Manage my reading')}>
          {TABS.map((item) => (
            <button key={item} type="button" role="tab" aria-selected={tab === item} className="ab-mode" data-testid={`ab-manage-tab-${item}`} onClick={() => controller.openManage(item)}>{labels[item]}</button>
          ))}
        </div>
      </header>
      {tab === 'sources' ? <SourcesTab /> : tab === 'goal' ? <GoalTab /> : <RecordsTab />}
    </section>
  )
}
