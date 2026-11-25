/* A page tidied by itself, in home mode: the page made for the address in the bar — when it was made and by whom
 * beside it — or that there is none yet and what the owner can do about it. The flow is reflow/controller.ts. */

import { Globe, X } from 'lucide-react'
import { useEffect, useMemo } from 'react'
import { LoadingOrb } from '../../../components/LoadingOrb'
import { useI18n } from '../../../i18n/provider'
import { ReflowFrame, type FramePage } from '../reflow/ReflowFrame'
import { formatMoment, useReflow, useReflowController } from '../reflow/useReflow'
import { useAgentBrowser } from '../store/context'
import '../reflow/reflow.css'

/** When the shown page was made and by whom (D23), and the way back to the original page. */
function Strip() {
  const controller = useReflowController()
  const { t, locale } = useI18n()
  const agents = useAgentBrowser((snapshot) => snapshot.agents)
  const state = useReflow((snapshot) => snapshot.state)
  const made = useReflow((snapshot) => (snapshot.view ? snapshot.state?.fallback ?? null : null))
  const maker = made ? agents.find((agent) => agent.agentDid === made.made_by)?.name ?? made.made_by : ''
  return (
    <div className="ab-reflow-strip" data-testid="ab-reflow-strip">
      <div className="ab-reflow-made">
        {made ? (
          <>
            <p className="ab-reflow-made-at" data-testid="ab-reflow-made" data-made-at={made.made_at}>
              {t('agentBrowser.reflow.madeAt', 'Made from the site as of {{time}}', { time: formatMoment(made.made_at, locale) })}
            </p>
            <p className="ab-reflow-made-by" data-testid="ab-reflow-maker" title={maker}>{t('agentBrowser.reflow.madeByFallback', 'By {{name}}, as a one-off page', { name: maker })}</p>
          </>
        ) : state ? <p className="ab-reflow-made-by">{new URL(state.url).hostname}</p> : null}
      </div>
      <button type="button" className="ab-button ab-reflow-source" aria-label={t('agentBrowser.reflow.source', 'Original page')} data-testid="ab-reflow-source" onClick={() => controller.openSource()}>
        <Globe size={18} aria-hidden /><span className="ab-reflow-source-label">{t('agentBrowser.reflow.source', 'Original page')}</span>
      </button>
    </div>
  )
}

function Notice() {
  const controller = useReflowController()
  const { t } = useI18n()
  const notice = useReflow((snapshot) => snapshot.notice)
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  if (!notice) return null
  return (
    <div className="ab-notes">
      <div key={notice.id} className="ab-note" role="status" data-testid="ab-reflow-notice" data-kind={notice.kind}>
        <p className="ab-note-text">
          {notice.kind === 'agent_asked'
            ? t('agentBrowser.reflow.notice.agentAsked', '{{name}} was asked. The page updates here when it is done.', { name })
            : t('agentBrowser.reflow.notice.agentUnreachable', '{{name}} could not be asked in the conversation right now.', { name })}
        </p>
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
  const reason = useReflow((snapshot) => snapshot.reason)
  const source = <button type="button" className="ab-button" data-testid="ab-reflow-status-source" onClick={() => controller.openSource()}>{t('agentBrowser.reflow.viewSource', 'View the original page')}</button>
  switch (phase) {
    case 'making':
      return (
        <div className="ab-message" role="status" data-testid="ab-reflow-making">
          <LoadingOrb size={36} />
          <p className="ab-message-title">{t('agentBrowser.reflow.making.title', 'Tidying this page')}</p>
          <p className="ab-message-detail">{t('agentBrowser.reflow.making.agent', '{{name}} was asked to make it. It shows here when it is done.', { name })}</p>
          <div className="ab-reflow-actions">
            {source}
            <button type="button" className="ab-button" data-testid="ab-reflow-ask-again" onClick={() => void controller.askAgent()}>{t('agentBrowser.reflow.making.askAgain', 'Ask again')}</button>
          </div>
        </div>
      )
    case 'unmade':
      return (
        <div className="ab-message" role={reason === 'not_made' ? undefined : 'alert'} data-testid="ab-reflow-unmade" data-reason={reason ?? undefined}>
          <p className="ab-message-title">{t('agentBrowser.reflow.unmade.title', 'This page has not been tidied yet')}</p>
          <p className="ab-message-detail">
            {reason === 'agent_unreachable'
              ? t('agentBrowser.reflow.unmade.agentUnreachable', '{{name}} could not be asked to make it: the conversation is not available here. Tell it the address in Chat, or try again.', { name })
              : t('agentBrowser.reflow.unmade.notMade', '{{name}} can make a version of this one page that is easy to read on your phone. The website is not added to your reading.', { name })}
          </p>
          <div className="ab-reflow-actions">
            <button type="button" className="ab-button ab-button-primary" data-testid="ab-reflow-ask" onClick={() => void controller.askAgent()}>{t('agentBrowser.reflow.askAgent', 'Ask {{name}}', { name })}</button>
            {source}
          </div>
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
          <p className="ab-message-title">{t('agentBrowser.reflow.failed.title', 'The page could not be looked up')}</p>
          <p className="ab-message-detail">{t('agentBrowser.reflow.failed.hint', 'Neither {{name}} nor your reflow library answered. Check your connection to the Zone, then try again.', { name })}</p>
          <div className="ab-reflow-actions">
            <button type="button" className="ab-button" data-testid="ab-reflow-retry" onClick={() => controller.retry()}>{t('common.retry')}</button>
            {source}
          </div>
        </div>
      )
    default:
      return <div className="ab-message" data-testid="ab-reflow-loading"><LoadingOrb size={36} /></div>
  }
}

const NO_POST = () => undefined

export function ReflowPanel() {
  const controller = useReflowController()
  const { t } = useI18n()
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  const phase = useReflow((snapshot) => snapshot.phase)
  const view = useReflow((snapshot) => snapshot.view)
  const frameKey = useReflow((snapshot) => snapshot.frameKey)
  const asked = useReflow((snapshot) => snapshot.asked)
  const address = useReflow((snapshot) => snapshot.state?.url ?? '')
  const page = useMemo((): FramePage => ({ ready: 'reflow.ready', onMessage: (data) => controller.onPageMessage(data), setPage: NO_POST }), [controller])
  useEffect(() => { controller.setTranslate(t) }, [controller, t])
  useEffect(() => {
    controller.activate()
    return () => controller.deactivate()
  }, [controller])
  return (
    <div className="ab-reflow" data-testid="ab-reflow-panel" data-phase={phase} data-url={address}>
      <Strip />
      <div className="ab-reflow-body">
        {phase === 'view' && view ? (
          <>
            <ReflowFrame view={view} frameKey={frameKey} page={page} title={t('agentBrowser.reflow.frameTitle', 'Reflow page')} />
            {asked ? <div className="ab-reflow-busy" role="status" data-testid="ab-reflow-busy">{t('agentBrowser.reflow.busy.agent', '{{name}} is making this page again…', { name })}</div> : null}
          </>
        ) : <Status />}
        <Notice />
      </div>
    </div>
  )
}
