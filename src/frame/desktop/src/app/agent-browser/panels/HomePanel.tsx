/* Home mode: the owner's personal site in a frame, with the shell's strip over it; before the site has a website,
 * the ways to start it; and a page tidied by itself when the owner opened one. Nothing here needs the browser: the
 * site is read from the owner's bucket and shown from `reflow.<zone>`. */

import { X } from 'lucide-react'
import { useEffect, useMemo } from 'react'
import { LoadingOrb } from '../../../components/LoadingOrb'
import { useI18n } from '../../../i18n/provider'
import { useThemeMode } from '../../../theme/provider'
import { ReflowFrame, type FramePage } from '../reflow/ReflowFrame'
import type { SiteNotice } from '../site/controller'
import { ManagePanel } from '../site/ManagePanel'
import { RequestSheet } from '../site/RequestSheet'
import { SiteStart, StartSheet } from '../site/SiteStart'
import { SiteStrip } from '../site/SiteStrip'
import { UpgradeSheet } from '../site/UpgradeSheet'
import { useSite, useSiteController } from '../site/useSite'
import { useAgentBrowser } from '../store/context'
import { ReflowPanel } from './ReflowPanel'
import '../site/site.css'

type Translate = ReturnType<typeof useI18n>['t']

function noticeText(t: Translate, notice: SiteNotice, name: string): string {
  const variables = { name, detail: notice.detail }
  switch (notice.kind) {
    case 'add_unsent':
      return t('agentBrowser.site.notice.addUnsent', '{{name}} could not be asked to add {{detail}}: the conversation is not available right now.', variables)
    case 'upgrade_unsent':
      return t('agentBrowser.site.notice.upgradeUnsent', '{{name}} could not be asked for the upgrade: the conversation is not available right now.', variables)
    case 'upgrade_unsaved':
      return t('agentBrowser.site.notice.upgradeUnsaved', 'Your answer could not be saved. Try again.')
    case 'add_again':
      return t('agentBrowser.site.notice.addAgain', '{{name}} was asked to try adding {{detail}} again.', variables)
    case 'remove_unsent':
      return t('agentBrowser.site.notice.removeUnsent', '{{name}} could not be asked to remove {{detail}}: the conversation is not available right now.', variables)
    case 'action_failed':
      return notice.detail ? t('agentBrowser.site.notice.actionFailed', 'That did not work: {{detail}}', variables) : t('agentBrowser.site.notice.actionFailedPlain', 'That did not work. Try again.')
    case 'session_failed':
      return t('agentBrowser.site.notice.sessionFailed', 'Your reading site could not be signed in to.')
  }
}

function Notice() {
  const site = useSiteController()
  const { t } = useI18n()
  const notice = useSite((snapshot) => snapshot.notice)
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  if (!notice) return null
  return (
    <div className="ab-notes">
      <div key={notice.id} className="ab-note" role={notice.kind === 'add_again' ? 'status' : 'alert'} data-testid="ab-site-notice" data-kind={notice.kind}>
        <p className="ab-note-text">{noticeText(t, notice, name)}</p>
        {notice.kind === 'session_failed' ? <button type="button" className="ab-button" data-testid="ab-site-session-retry" onClick={() => site.reopen()}>{t('common.retry')}</button> : null}
        <button type="button" className="ab-icon-button" aria-label={t('agentBrowser.dismiss', 'Dismiss')} data-testid="ab-site-notice-dismiss" onClick={() => site.dismissNotice()}><X size={16} aria-hidden /></button>
      </div>
    </div>
  )
}

/** `hidden`: another mode is on the stage; what was being read stays where it was for the way back. */
export function HomePanel({ hidden }: { hidden: boolean }) {
  const site = useSiteController()
  const { t } = useI18n()
  const status = useSite((snapshot) => snapshot.status)
  const place = useSite((snapshot) => snapshot.place)
  const frame = useSite((snapshot) => snapshot.frame)
  const sheet = useSite((snapshot) => snapshot.sheet)
  const manage = useSite((snapshot) => snapshot.manage)
  const started = useSite((snapshot) => (snapshot.site?.sources.length ?? 0) > 0)
  const { themeMode } = useThemeMode()
  const page = useMemo((): FramePage => ({ ready: 'site.ready', awaitReady: true, onMessage: (data) => site.onPageMessage(data), setPage: (post) => site.setPage(post) }), [site])
  useEffect(() => {
    site.setShown(!hidden)
    return () => site.setShown(false)
  }, [site, hidden])
  useEffect(() => { site.setTheme(themeMode) }, [site, themeMode])
  const tidied = place.kind === 'page'
  const closeSheet = () => site.openSheet(null)
  return (
    <div className="ab-home" hidden={hidden} data-testid="ab-home" data-status={status} data-place={place.kind} data-route={place.kind === 'site' ? place.route : undefined}>
      {tidied ? <ReflowPanel /> : null}
      <div className="ab-home-site" hidden={tidied}>
        {status === 'loading' ? <div className="ab-message" data-testid="ab-site-loading"><LoadingOrb size={36} /></div>
          : status === 'failed' ? (
            <div className="ab-message" role="alert" data-testid="ab-site-failed">
              <p className="ab-message-title">{t('agentBrowser.site.failed.title', 'Your reading site could not be read')}</p>
              <p className="ab-message-detail">{t('agentBrowser.site.failed.hint', 'Check your connection to the Zone, then try again.')}</p>
              <button type="button" className="ab-button" data-testid="ab-site-retry" onClick={() => void site.load()}>{t('common.retry')}</button>
            </div>
          ) : started ? (
            <>
              <SiteStrip />
              <div className="ab-home-body">
                <ReflowFrame view={frame.view ? `?${frame.view}` : ''} frameKey={frame.key} page={page} title={t('agentBrowser.site.frameTitle', 'Reading site')} testId="ab-site-frame" />
              </div>
            </>
          ) : <SiteStart />}
      </div>
      {manage ? <ManagePanel tab={manage} /> : null}
      <Notice />
      {sheet?.kind === 'add' ? <RequestSheet kind="add" url={sheet.url} title={sheet.title} onClose={closeSheet} />
        : sheet?.kind === 'start' ? <StartSheet onClose={closeSheet} />
        : sheet?.kind === 'upgrade' ? <UpgradeSheet id={sheet.id} onClose={closeSheet} />
        : null}
    </div>
  )
}
