/* The App's frame, designed for a phone and used as it is on the desktop: the thing and the mode on top, the
 * address under them, the stage (the screen, or the reflow / conversation panel), and the strip that says who
 * operates. Before that: choosing one of the user's own Agents (D10). */

import { ChevronDown, EllipsisVertical, MonitorOff } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { LoadingOrb } from '../../../components/LoadingOrb'
import { useI18n } from '../../../i18n/provider'
import { openAgentSetup } from '../../agent-setup/launch'
import { HubAvatar } from '../../messagehub/HubAvatar'
import { useViewerIsAdmin } from '../../settings/hooks/use-viewer-is-admin'
import { openSettings } from '../../settings/launch'
import { ConversationSheet } from '../conversation/ConversationSheet'
import { browserModes } from '../launch'
import { ConversationPanel } from '../panels/ConversationPanel'
import { ReflowPanel } from '../panels/ReflowPanel'
import { ReflowAddressBar } from '../reflow/ReflowAddressBar'
import { RequestLayer } from '../requests/RequestLayer'
import { ScreenView } from '../screen/ScreenView'
import { useAgentBrowser, useAgentBrowserStore } from '../store/context'
import { browserUsable, canOperate, currentThing } from '../store/store'
import { thingTitle } from '../store/things'
import { AddressBar } from './AddressBar'
import { BrowserDownload } from './BrowserDownload'
import { HandoverBar } from './HandoverBar'
import { MenuSheet, TabsSheet, ThingsSheet } from './sheets'

function Message({ testId, title, detail, action }: { testId: string; title: string; detail?: string; action?: { label: string; onClick: () => void; testId: string } }) {
  return (
    <div className="ab-message" data-testid={testId}>
      <p className="ab-message-title">{title}</p>
      {detail ? <p className="ab-message-detail">{detail}</p> : null}
      {action ? <button type="button" className="ab-button ab-button-primary" data-testid={action.testId} onClick={action.onClick}>{action.label}</button> : null}
    </div>
  )
}

function AgentPicker() {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const agents = useAgentBrowser((snapshot) => snapshot.agents)
  return (
    <div className="ab-picker" data-testid="ab-agent-picker">
      <h2 className="ab-picker-title">{t('agentBrowser.agent.pick', 'Whose browser do you want to use?')}</h2>
      <p className="ab-picker-hint">{t('agentBrowser.agent.pickHint', 'Each of your Agents has its own browser, with its own tabs and sign-ins.')}</p>
      <ul className="ab-list">
        {agents.map((agent) => (
          <li key={agent.agentId}>
            <button type="button" className="ab-list-item ab-picker-item" data-testid={`ab-agent-${agent.agentId}`} onClick={() => store.selectAgent(agent.agentId)}>
              <HubAvatar type="agent" name={agent.name} avatar={agent.avatar} size={40} />
              <span className="ab-list-text">
                <span className="ab-list-title">{agent.name}</span>
                <span className="ab-list-detail">{agent.agentId}</span>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  )
}

/** Why the browser cannot be used on this Agent, in the user's terms, with the service's own sentence under it. */
function Unavailable() {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const browser = useAgentBrowser((snapshot) => snapshot.browser)
  const failed = useAgentBrowser((snapshot) => snapshot.browserError)
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  const isAdmin = useViewerIsAdmin()
  if (!browser) {
    return failed
      ? <Message testId="ab-unreachable" title={t('agentBrowser.unavailable.unreachable', '{{name}} cannot be reached right now.', { name })} detail={t('agentBrowser.unavailable.unreachableHint', 'Its service did not answer. Check that the Agent is running, then try again.')} action={{ label: t('common.retry'), onClick: () => void store.refreshStatus(), testId: 'ab-status-retry' }} />
      : <div className="ab-message" data-testid="ab-status-loading"><LoadingOrb size={36} /></div>
  }
  const reason = browser.unavailable_code ?? (browser.enabled ? 'no_package' : 'disabled')
  const detail = {
    disabled: t('agentBrowser.unavailable.disabled', 'The browser is turned off in this Agent’s configuration ([loader.browser] enabled = false).'),
    no_package: t('agentBrowser.unavailable.noPackage', 'The node this Agent runs on does not have the “ext-core” extension tools, which bring the browser.'),
    not_downloaded: t('agentBrowser.unavailable.notDownloaded', 'The browser is not downloaded to this node yet.'),
  }[reason]
  return (
    <div className="ab-message ab-unavailable" role="alert" data-testid="ab-unavailable" data-reason={reason}>
      <MonitorOff size={28} aria-hidden />
      <p className="ab-message-title">{t('agentBrowser.unavailable.title', 'The browser is not available on {{name}}', { name })}</p>
      <p className="ab-message-detail">{detail}</p>
      {reason === 'not_downloaded' && isAdmin ? <BrowserDownload /> : null}
      {reason === 'not_downloaded' && isAdmin === false ? <p className="ab-message-detail" data-testid="ab-ask-admin">{t('agentBrowser.unavailable.notDownloadedAsk', 'An administrator can download it in Settings › Extension tools. The browser connects here by itself once it is there.')}</p> : null}
      {reason === 'no_package' && isAdmin ? <button type="button" className="ab-button ab-button-primary" data-testid="ab-open-ext-tools" onClick={() => openSettings('ext-tools')}>{t('agentBrowser.unavailable.openExtTools', 'Open Extension tools')}</button> : null}
      {reason === 'no_package' && isAdmin === false ? <p className="ab-message-detail" data-testid="ab-ask-admin">{t('agentBrowser.unavailable.noPackageAsk', 'Ask an administrator to enable ext-core in Settings › Extension tools.')}</p> : null}
      {browser.unavailable ? <p className="ab-message-raw" data-testid="ab-unavailable-detail">{browser.unavailable}</p> : null}
      <button type="button" className="ab-button" data-testid="ab-status-retry" onClick={() => void store.refreshStatus()}>{t('common.retry')}</button>
    </div>
  )
}

function Toast() {
  const { t } = useI18n()
  const notice = useAgentBrowser((snapshot) => snapshot.notice)
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  if (!notice) return null
  const text = notice.kind === 'agent_in_control' ? t('agentBrowser.notice.agentInControl', '{{name}} is operating. Take over to use the page.', { name })
    : notice.kind === 'offline' ? t('agentBrowser.notice.offline', 'The browser is not connected yet.')
    : notice.kind === 'profile_cleared' ? t('agentBrowser.notice.profileCleared', 'Sign-ins cleared.')
    : t(`agentBrowser.error.${notice.code ?? 'unknown'}`, t('agentBrowser.error.unknown', 'That did not work. Try again.'))
  return <div key={notice.id} className="ab-toast" role="status" data-testid="ab-toast" data-kind={notice.kind} data-code={notice.code}>{text}</div>
}

type SheetName = 'things' | 'tabs' | 'menu'

function BrowserFrame() {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const agent = useAgentBrowser((snapshot) => snapshot.agent)
  const mode = useAgentBrowser((snapshot) => snapshot.mode)
  const groupId = useAgentBrowser((snapshot) => snapshot.groupId)
  const things = useAgentBrowser((snapshot) => snapshot.things)
  const usable = useAgentBrowser(browserUsable)
  const operable = useAgentBrowser(canOperate)
  const loading = useAgentBrowser((snapshot) => snapshot.state?.loading ?? false)
  const [sheet, setSheet] = useState<SheetName | null>(null)
  const stageRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const stage = stageRef.current
    if (!stage) return
    const observer = new ResizeObserver(() => store.setStageSize({ width: stage.clientWidth, height: stage.clientHeight }))
    observer.observe(stage)
    return () => observer.disconnect()
  }, [store])
  const thing = currentThing({ things, groupId })
  const close = () => setSheet(null)
  return (
    <div className="ab-root" data-testid="agent-browser" data-mode={mode} data-group={groupId} data-agent={agent?.agentId} data-operable={operable ? 'true' : 'false'}>
      <div className="ab-top">
        <button type="button" className="ab-thing-button" aria-haspopup="dialog" data-testid="ab-thing-button" onClick={() => setSheet('things')}>
          {agent ? <HubAvatar type="agent" name={agent.name} avatar={agent.avatar} size={24} /> : null}
          <span className="ab-thing-names">
            <span className="ab-thing-agent" data-testid="ab-agent-name">{agent?.name}</span>
            <span className="ab-thing-title" data-testid="ab-thing-title">{thingTitle(t, thing)}</span>
          </span>
          <ChevronDown size={16} aria-hidden />
        </button>
        <div className="ab-modes" role="tablist" aria-label={t('agentBrowser.mode.label', 'View')}>
          {browserModes.map((item) => (
            <button key={item} type="button" role="tab" aria-selected={mode === item} className="ab-mode" data-testid={`ab-mode-${item}`} onClick={() => store.setMode(item)}>
              {t(`agentBrowser.mode.${item}`, item)}
            </button>
          ))}
        </div>
        <button type="button" className="ab-icon-button" aria-label={t('agentBrowser.menu.title', 'Browser settings')} aria-haspopup="dialog" data-testid="ab-menu-button" onClick={() => setSheet('menu')}><EllipsisVertical size={18} aria-hidden /></button>
      </div>
      {mode === 'chat' ? null : mode === 'reflow' ? <ReflowAddressBar /> : <AddressBar onOpenTabs={() => setSheet('tabs')} />}
      <div className="ab-progress" data-active={loading && mode === 'page' ? 'true' : 'false'} aria-hidden />
      <div ref={stageRef} className="ab-stage" data-testid="ab-stage" role="tabpanel">
        {mode === 'reflow' ? <ReflowPanel /> : mode === 'chat' ? <ConversationPanel /> : usable ? <ScreenView key={groupId} /> : <Unavailable />}
        {usable && mode !== 'reflow' ? <RequestLayer /> : null}
        {mode === 'chat' ? null : <ConversationSheet />}
        <Toast />
      </div>
      <HandoverBar />
      {sheet === 'things' ? <ThingsSheet onClose={close} /> : sheet === 'tabs' ? <TabsSheet onClose={close} /> : sheet === 'menu' ? <MenuSheet onClose={close} /> : null}
    </div>
  )
}

export function AgentBrowserShell() {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const phase = useAgentBrowser((snapshot) => snapshot.phase)
  switch (phase) {
    case 'loading':
      return <div className="ab-root ab-center" data-testid="agent-browser-loading"><LoadingOrb size={44} /></div>
    case 'error':
      return <div className="ab-root ab-center"><Message testId="agent-browser-error" title={t('agentBrowser.agent.loadFailed', 'Your Agents could not be loaded.')} action={{ label: t('common.retry'), onClick: () => store.retry(), testId: 'ab-retry' }} /></div>
    case 'empty':
      return <div className="ab-root ab-center"><Message testId="agent-browser-empty" title={t('agentBrowser.agent.none', 'You have no Agent yet')} detail={t('agentBrowser.agent.noneHint', 'The browser belongs to an Agent. Create one first, then open its browser here.')} action={{ label: t('agentBrowser.agent.create', 'Create an Agent'), onClick: () => { openAgentSetup({ source: 'users_agents' }) }, testId: 'ab-create-agent' }} /></div>
    case 'pick':
      return <div className="ab-root" data-testid="agent-browser"><AgentPicker /></div>
    case 'agent':
      return <BrowserFrame />
  }
}
