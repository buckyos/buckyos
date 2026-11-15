/* Reflow mode (RF6): the stage shows the page made for the address in the bar — when it was made and by whom
 * beside it — or what is being done to make one. The flow is reflow/controller.ts. */

import { Globe, LayoutList, SlidersHorizontal, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import { LoadingOrb } from '../../../components/LoadingOrb'
import { useI18n } from '../../../i18n/provider'
import type { ReflowNotice } from '../reflow/controller'
import { shownMade } from '../reflow/library'
import { PreferenceSheet } from '../reflow/PreferenceSheet'
import { ReflowFrame } from '../reflow/ReflowFrame'
import { formatMoment, useReflow, useReflowController } from '../reflow/useReflow'
import { useAgentBrowser } from '../store/context'
import '../reflow/reflow.css'

type Translate = ReturnType<typeof useI18n>['t']

/** When the shown page was made and by whom (D23), the site's preference, and the way back to the original page. */
function Strip({ onPreference }: { onPreference: () => void }) {
  const controller = useReflowController()
  const { t, locale } = useI18n()
  const agents = useAgentBrowser((snapshot) => snapshot.agents)
  const state = useReflow((snapshot) => snapshot.state)
  const shown = useReflow((snapshot) => snapshot.view !== null)
  const preference = useReflow((snapshot) => snapshot.preference)
  const made = state && shown ? shownMade(state) : null
  const fromPackage = !!state?.package && !!state.result
  const maker = made ? agents.find((agent) => agent.agentDid === made.made_by)?.name ?? made.made_by : ''
  return (
    <div className="ab-reflow-strip" data-testid="ab-reflow-strip">
      <div className="ab-reflow-made">
        {made ? (
          <>
            <p className="ab-reflow-made-at" data-testid="ab-reflow-made" data-made-at={made.made_at} data-kind={fromPackage ? 'package' : 'fallback'}>
              {t('agentBrowser.reflow.madeAt', 'Made from the site as of {{time}}', { time: formatMoment(made.made_at, locale) })}
            </p>
            <p className="ab-reflow-made-by" data-testid="ab-reflow-maker" title={maker}>
              {fromPackage ? t('agentBrowser.reflow.madeBy', 'By {{name}}, with the site’s reflow package', { name: maker }) : t('agentBrowser.reflow.madeByFallback', 'By {{name}}, as a one-off page', { name: maker })}
            </p>
          </>
        ) : state ? <p className="ab-reflow-made-by">{new URL(state.url).hostname}</p> : null}
      </div>
      {state ? (
        <button type="button" className="ab-icon-button" aria-label={t('agentBrowser.reflow.preference.open', 'Preference for this site')} aria-haspopup="dialog" data-testid="ab-reflow-preference" data-set={preference ? 'true' : 'false'} onClick={onPreference}>
          <SlidersHorizontal size={18} aria-hidden />
        </button>
      ) : null}
      <button type="button" className="ab-button ab-reflow-source" aria-label={t('agentBrowser.reflow.source', 'Original page')} data-testid="ab-reflow-source" onClick={() => controller.openSource()}>
        <Globe size={18} aria-hidden /><span className="ab-reflow-source-label">{t('agentBrowser.reflow.source', 'Original page')}</span>
      </button>
    </div>
  )
}

function noticeText(t: Translate, notice: ReflowNotice, name: string): string {
  const variables = { name, detail: notice.detail }
  const asked = notice.asked ? ` ${t('agentBrowser.reflow.notice.handed', '{{name}} was asked to take care of it.', variables)}` : ''
  switch (notice.kind) {
    case 'browser_unavailable':
      return t('agentBrowser.reflow.notice.browserUnavailable', '{{name}}’s browser is not available right now, so the page cannot be made again. What was already made can still be read.', variables)
    case 'refresh_failed':
      return (notice.detail ? t('agentBrowser.reflow.notice.refreshFailed', 'Making the page again failed: {{detail}}', variables) : t('agentBrowser.reflow.notice.refreshFailedPlain', 'Making the page again failed.')) + asked
    case 'action_failed':
      return (notice.detail ? t('agentBrowser.reflow.notice.actionFailed', 'That could not be done on the original page: {{detail}}', variables) : t('agentBrowser.reflow.notice.actionFailedPlain', 'That could not be done on the original page.')) + asked
    case 'agent_asked':
      return t('agentBrowser.reflow.notice.agentAsked', '{{name}} was asked. The page updates here when it is done.', variables)
    case 'agent_unreachable':
      return t('agentBrowser.reflow.notice.agentUnreachable', '{{name}} could not be asked in the conversation right now.', variables)
    case 'preference_failed':
      return t('agentBrowser.reflow.notice.preferenceFailed', 'The preference was not saved. Try again.')
  }
}

function Notice() {
  const controller = useReflowController()
  const { t } = useI18n()
  const notice = useReflow((snapshot) => snapshot.notice)
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  if (!notice) return null
  return (
    <div className="ab-notes">
      <div key={notice.id} className="ab-note" role="status" data-testid="ab-reflow-notice" data-kind={notice.kind} data-asked={notice.asked ? 'true' : 'false'}>
        <p className="ab-note-text">{noticeText(t, notice, name)}</p>
        <button type="button" className="ab-icon-button" aria-label={t('agentBrowser.dismiss', 'Dismiss')} data-testid="ab-reflow-notice-dismiss" onClick={() => controller.dismissNotice()}><X size={16} aria-hidden /></button>
      </div>
    </div>
  )
}

/** What the stage says while there is no page to show. */
function Status() {
  const controller = useReflowController()
  const { t } = useI18n()
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  const phase = useReflow((snapshot) => snapshot.phase)
  const job = useReflow((snapshot) => snapshot.job)
  const reason = useReflow((snapshot) => snapshot.reason)
  const failure = useReflow((snapshot) => snapshot.failure)
  const source = <button type="button" className="ab-button" data-testid="ab-reflow-status-source" onClick={() => controller.openSource()}>{t('agentBrowser.reflow.viewSource', 'View the original page')}</button>
  const retry = <button type="button" className="ab-button" data-testid="ab-reflow-retry" onClick={() => controller.retry()}>{t('common.retry')}</button>
  const failed = failure ? <p className="ab-message-raw" data-testid="ab-reflow-failure">{failure}</p> : null
  switch (phase) {
    case 'blank':
      return (
        <div className="ab-message" data-testid="ab-reflow-blank">
          <LayoutList size={28} aria-hidden />
          <p className="ab-message-title">{t('agentBrowser.reflow.blank.title', 'Enter an address to read it reflowed')}</p>
          <p className="ab-message-detail">{t('agentBrowser.reflow.blank.hint', 'A reflowed page shows a site the way you prefer. It is made ahead of time, so it opens without the browser running.')}</p>
        </div>
      )
    case 'making':
      return (
        <div className="ab-message" role="status" data-testid="ab-reflow-making" data-by={job ? 'refresh' : 'agent'}>
          <LoadingOrb size={36} />
          <p className="ab-message-title">{t('agentBrowser.reflow.making.title', 'Making the reflowed page')}</p>
          <p className="ab-message-detail">
            {job
              ? t('agentBrowser.reflow.making.refresh', '{{name}}’s browser is making it with this site’s reflow package.', { name })
              : failure
                ? t('agentBrowser.reflow.making.agentAfterFailure', 'The site’s reflow package could not make it. {{name}} was asked to make this page and repair the package.', { name })
                : t('agentBrowser.reflow.making.agent', '{{name}} was asked to make it. It shows here when it is done.', { name })}
          </p>
          {failed}
          <div className="ab-reflow-actions">
            {source}
            {job ? null : <button type="button" className="ab-button" data-testid="ab-reflow-ask-again" onClick={() => void controller.askAgain()}>{t('agentBrowser.reflow.making.askAgain', 'Ask again')}</button>}
          </div>
        </div>
      )
    case 'unmade':
      return (
        <div className="ab-message" role="alert" data-testid="ab-reflow-unmade" data-reason={reason ?? undefined}>
          <p className="ab-message-title">{t('agentBrowser.reflow.unmade.title', 'This page has not been reflowed yet')}</p>
          <p className="ab-message-detail">
            {reason === 'browser_unavailable'
              ? t('agentBrowser.reflow.unmade.browserUnavailable', '{{name}}’s browser is not available right now, so the page cannot be made. Pages that were already made can still be read.', { name })
              : reason === 'not_started'
                ? t('agentBrowser.reflow.unmade.notStarted', 'Making it could not be started. Try again.')
                : t('agentBrowser.reflow.unmade.agentUnreachable', '{{name}} could not be asked to make it: the conversation is not available here. Tell it the address in Chat, or try again.', { name })}
          </p>
          {failed}
          <div className="ab-reflow-actions">{retry}{source}</div>
        </div>
      )
    case 'unsupported':
      return (
        <div className="ab-message" data-testid="ab-reflow-unsupported">
          <p className="ab-message-title">{t('agentBrowser.reflow.unsupported.title', 'This address cannot be reflowed')}</p>
          <p className="ab-message-detail">{t('agentBrowser.reflow.unsupported.hint', 'Only web pages (http and https addresses) are reflowed.')}</p>
          <div className="ab-reflow-actions">{source}</div>
        </div>
      )
    case 'failed':
      return (
        <div className="ab-message" role="alert" data-testid="ab-reflow-failed">
          <p className="ab-message-title">{t('agentBrowser.reflow.failed.title', 'The reflow library could not be read')}</p>
          <p className="ab-message-detail">{t('agentBrowser.reflow.failed.hint', '{{name}} did not answer. Check that the Agent is running, then try again.', { name })}</p>
          <div className="ab-reflow-actions">{retry}{source}</div>
        </div>
      )
    default:
      return <div className="ab-message" data-testid="ab-reflow-loading"><LoadingOrb size={36} /></div>
  }
}

/** While a page is shown: what is running for it. */
function Busy() {
  const { t } = useI18n()
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  const job = useReflow((snapshot) => snapshot.job)
  const asked = useReflow((snapshot) => snapshot.asked)
  if (!job && !asked) return null
  return (
    <div className="ab-reflow-busy" role="status" data-testid="ab-reflow-busy" data-job={job ?? 'agent'}>
      {job === 'action' ? t('agentBrowser.reflow.busy.action', 'Doing it on the original page…')
        : job === 'refresh' ? t('agentBrowser.reflow.busy.refresh', 'Making the page again…')
        : t('agentBrowser.reflow.busy.agent', '{{name}} is making this page again…', { name })}
    </div>
  )
}

export function ReflowPanel() {
  const controller = useReflowController()
  const { t } = useI18n()
  const phase = useReflow((snapshot) => snapshot.phase)
  const view = useReflow((snapshot) => snapshot.view)
  const frameKey = useReflow((snapshot) => snapshot.frameKey)
  const address = useReflow((snapshot) => snapshot.state?.url ?? '')
  const [preferenceOpen, setPreferenceOpen] = useState(false)
  useEffect(() => { controller.setTranslate(t) }, [controller, t])
  useEffect(() => {
    controller.activate()
    return () => controller.deactivate()
  }, [controller])
  return (
    <div className="ab-reflow" data-testid="ab-reflow-panel" data-phase={phase} data-url={address}>
      {phase === 'blank' ? null : <Strip onPreference={() => setPreferenceOpen(true)} />}
      <div className="ab-reflow-body">
        {phase === 'view' && view ? <><ReflowFrame view={view} frameKey={frameKey} /><Busy /></> : <Status />}
        <Notice />
      </div>
      {preferenceOpen ? <PreferenceSheet key={address} onClose={() => setPreferenceOpen(false)} /> : null}
    </div>
  )
}
