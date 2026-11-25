/* What the shell says about the page of the personal site being read (Personal Site Protocol §6.2: this is the
 * shell's, read from `_data`, whatever the site's template shows): the source and when it was last synced, that it
 * needs a sign-in or an upgrade, the way to the original page — and, at the site's front page, the websites being
 * added and who maintains the site. */

import { Globe, SlidersHorizontal, X } from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { useAgentBrowser, useAgentBrowserStore } from '../store/context'
import { currentSource, sourceOfAdd, type PendingAdd } from './controller'
import { displayState, openUpgrades, type Feed, type Upgrade } from './data'
import { SOURCE_TONES, sourceText } from './status'
import { useSite, useSiteController } from './useSite'

type Translate = ReturnType<typeof useI18n>['t']

function pendingText(t: Translate, pending: NonNullable<Feed['status']['pending']>): string {
  const variables = { count: pending.count }
  switch (pending.reason) {
    case 'translate_budget':
      return t('agentBrowser.site.pending.translateBudget', '{{count}} new articles wait for translation: today’s budget is used up', variables)
    case 'translate_config':
      return t('agentBrowser.site.pending.translateConfig', '{{count}} new articles wait for translation: no model is available for it', variables)
    case 'images':
      return t('agentBrowser.site.pending.images', '{{count}} new articles wait for their pictures', variables)
    default:
      return t('agentBrowser.site.pending.time', '{{count}} new articles come with the next sync', variables)
  }
}

function UpgradeNote({ upgrade }: { upgrade: Upgrade }) {
  const site = useSiteController()
  const { t } = useI18n()
  return (
    <div className="ab-site-note" data-tone="attention" data-testid="ab-site-upgrade" data-upgrade={upgrade.id}>
      <span className="ab-site-dot" aria-hidden />
      <div className="ab-site-text">
        <p className="ab-site-main">{upgrade.title}</p>
        <p className="ab-site-sub ab-site-clamp">{upgrade.summary}</p>
      </div>
      <button type="button" className="ab-button" aria-haspopup="dialog" data-testid="ab-site-upgrade-open" onClick={() => site.openSheet({ kind: 'upgrade', id: upgrade.id })}>{t('agentBrowser.site.upgrade.review', 'Review')}</button>
    </div>
  )
}

function AddNote({ add }: { add: PendingAdd }) {
  const site = useSiteController()
  const { t } = useI18n()
  const agents = useAgentBrowser((snapshot) => snapshot.agents)
  const source = useSite((snapshot) => sourceOfAdd(snapshot.site, add))
  const name = agents.find((agent) => agent.agentId === add.agentId)?.name ?? add.agentId
  const state = add.failed ? 'unsent' : source ? 'preparing' : add.state
  const main = source
    ? t('agentBrowser.site.add.preparing', '{{name}} is preparing {{site}}', { name, site: source.title })
    : add.state === 'asked'
      ? t('agentBrowser.site.add.asked', 'Asked {{name}} to add {{site}}', { name, site: add.host })
      : t('agentBrowser.site.add.queued', '{{site}} waits for its turn', { site: add.host })
  const sub = add.failed
    ? t('agentBrowser.site.add.unsent', 'The request could not be sent.')
    : add.state === 'asked'
      ? t('agentBrowser.site.add.waiting', 'It shows here when the first articles are ready. You can keep reading.')
      : t('agentBrowser.site.add.queuedHint', 'Websites are added one at a time.')
  return (
    <div className="ab-site-note" data-tone={add.failed ? 'failed' : add.state === 'asked' ? 'busy' : 'idle'} role="status" data-testid="ab-site-add" data-key={add.key} data-state={state}>
      <span className="ab-site-dot" aria-hidden />
      <div className="ab-site-text">
        <p className="ab-site-main">{main}</p>
        <p className="ab-site-sub">{sub}</p>
      </div>
      {add.failed ? <button type="button" className="ab-button" data-testid="ab-site-add-retry" onClick={() => site.retryAdd(add.key)}>{t('common.retry')}</button> : null}
      <button type="button" className="ab-icon-button" aria-label={t('agentBrowser.site.add.remove', 'Stop waiting for {{site}}', { site: add.host })} data-testid="ab-site-add-remove" onClick={() => site.removeAdd(add.key)}><X size={16} aria-hidden /></button>
    </div>
  )
}

/** What concerns the whole site: websites being added, upgrade requests about no single source, and who can maintain it. */
export function SiteNotes() {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const agent = useAgentBrowser((snapshot) => snapshot.agent)
  const agents = useAgentBrowser((snapshot) => snapshot.agents)
  const site = useSite((snapshot) => snapshot.site)
  const adds = useSite((snapshot) => snapshot.adds)
  if (!agent) return null
  const maintainer = site?.maintainer && site.maintainer.agent !== agent.agentDid ? site.maintainer.agent : null
  const other = maintainer ? agents.find((item) => item.agentDid === maintainer) : undefined
  return (
    <>
      {adds.map((add) => <AddNote key={add.key} add={add} />)}
      {site ? openUpgrades(site, null).map((upgrade) => <UpgradeNote key={upgrade.id} upgrade={upgrade} />) : null}
      {maintainer ? (
        <div className="ab-site-note" data-tone="idle" data-testid="ab-site-maintainer">
          <span className="ab-site-dot" aria-hidden />
          <div className="ab-site-text">
            <p className="ab-site-sub">{t('agentBrowser.site.otherMaintainer', 'This site is maintained by {{maintainer}}, not by {{name}}.', { maintainer: other?.name ?? maintainer, name: agent.name })}</p>
          </div>
          {other ? <button type="button" className="ab-button" data-testid="ab-site-maintainer-switch" onClick={() => store.selectAgent(other.agentId)}>{t('agentBrowser.site.switchAgent', 'Switch')}</button> : null}
        </div>
      ) : null}
      {agent.readOnly ? (
        <div className="ab-site-note" data-tone="idle" data-testid="ab-site-read-only">
          <span className="ab-site-dot" aria-hidden />
          <div className="ab-site-text">
            <p className="ab-site-sub">{t('agentBrowser.site.readOnly', '{{name}} has read-only access, so it cannot keep a reading site: it cannot save what it collects.', { name: agent.name })}</p>
          </div>
        </div>
      ) : null}
    </>
  )
}

export function SiteStrip() {
  const controller = useSiteController()
  const store = useAgentBrowserStore()
  const { t, locale } = useI18n()
  const site = useSite((snapshot) => snapshot.site)
  const source = useSite(currentSource)
  const feed = useSite((snapshot) => (source ? snapshot.feeds[source.id] ?? null : null))
  const article = useSite((snapshot) => snapshot.article)
  const atFront = useSite((snapshot) => snapshot.place.kind === 'site' && snapshot.place.route === '')
  if (!site) return null
  const state = source ? displayState(site, source, feed) : null
  const original = article?.url ?? source?.home_url ?? ''
  const login = state === 'needs_login' ? feed?.status.login_url ?? source?.home_url ?? '' : ''
  return (
    <div className="ab-site-strip" data-testid="ab-site-strip" data-source={source?.id} data-state={state ?? undefined}>
      <div className="ab-site-row" data-tone={state ? SOURCE_TONES[state] : undefined}>
        {state ? <span className="ab-site-dot" aria-hidden /> : null}
        <div className="ab-site-text">
          <p className="ab-site-main" data-testid="ab-site-title">{source?.title ?? (site.title || t('agentBrowser.site.title', 'My reading'))}</p>
          <p className="ab-site-sub" data-testid="ab-site-status" data-checked-at={feed?.status.checked_at ?? undefined}>
            {source && state ? sourceText(t, locale, state, source, feed) : t('agentBrowser.site.sources', 'Websites: {{count}}', { count: site.sources.length })}
          </p>
        </div>
        {login ? <button type="button" className="ab-button ab-button-primary" data-testid="ab-site-sign-in" onClick={() => store.openInPage(login)}>{t('agentBrowser.site.signIn', 'Sign in')}</button> : null}
        {original ? (
          <button type="button" className="ab-button ab-reflow-source" aria-label={t('agentBrowser.reflow.source', 'Original page')} data-testid="ab-site-original" onClick={() => store.openInPage(original)}>
            <Globe size={18} aria-hidden /><span className="ab-reflow-source-label">{t('agentBrowser.reflow.source', 'Original page')}</span>
          </button>
        ) : null}
        <button type="button" className="ab-icon-button" aria-label={t('agentBrowser.manage.title', 'Manage my reading')} aria-haspopup="dialog" data-testid="ab-site-manage" onClick={() => controller.openManage('sources')}><SlidersHorizontal size={18} aria-hidden /></button>
      </div>
      {source && state === 'failed' && source.state !== 'failed' && feed?.status.error?.message ? <p className="ab-site-raw" data-testid="ab-site-error">{feed.status.error.message}</p> : null}
      {source && feed?.status.pending ? <p className="ab-site-more" data-testid="ab-site-pending" data-reason={feed.status.pending.reason}>{pendingText(t, feed.status.pending)}</p> : null}
      {source ? openUpgrades(site, source.id).map((upgrade) => <UpgradeNote key={upgrade.id} upgrade={upgrade} />) : null}
      {atFront ? <SiteNotes /> : null}
    </div>
  )
}
