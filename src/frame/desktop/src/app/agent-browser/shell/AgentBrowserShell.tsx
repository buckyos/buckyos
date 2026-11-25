/* The App's frame, designed for a phone: the Agent and the mode on top, the address under them, the stage (the
 * personal site of home mode, the screen, or the conversation), and the strip that says who operates. On a desktop
 * the top is one row — the Agent, the address, the mode, the tabs, the menu — unless the window is too narrow for
 * it. Before that: choosing one of the user's own Agents (D10). */

import { useMediaQuery } from '@mui/material'
import { EllipsisVertical, MonitorOff } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { LoadingOrb } from '../../../components/LoadingOrb'
import { useI18n } from '../../../i18n/provider'
import { openAgentSetup } from '../../agent-setup/launch'
import { HubAvatar } from '../../messagehub/HubAvatar'
import { useAppPath } from '../../pathContext'
import { appPath } from '../../standalone/address'
import { useViewerIsAdmin } from '../../settings/hooks/use-viewer-is-admin'
import { openSettings } from '../../settings/launch'
import { ConversationSheet } from '../conversation/ConversationSheet'
import { browserModes } from '../launch'
import { ConversationPanel } from '../panels/ConversationPanel'
import { HomePanel } from '../panels/HomePanel'
import { RequestLayer } from '../requests/RequestLayer'
import { ScreenView } from '../screen/ScreenView'
import { HomeAddressBar } from '../site/HomeAddressBar'
import { useSiteController } from '../site/useSite'
import { useAgentBrowser, useAgentBrowserStore } from '../store/context'
import { browserUsable, canOperate } from '../store/store'
import { AddressBar, TabsButton } from './AddressBar'
import { BrowserDownload } from './BrowserDownload'
import { HandoverBar } from './HandoverBar'
import { MenuSheet, TabsSheet } from './sheets'

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
    no_display: t('agentBrowser.unavailable.noDisplay', 'This Agent’s runtime has no virtual display for the browser to run on. Its container image is older than the browser needs: update BuckyOS on this node.'),
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

type SheetName = 'tabs' | 'menu'

/** Below this width of the App the one-row top of a desktop leaves the address too little room: it goes back to two rows. */
const ONE_ROW_MIN_WIDTH = 640

function BrowserFrame() {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const agent = useAgentBrowser((snapshot) => snapshot.agent)
  const mode = useAgentBrowser((snapshot) => snapshot.mode)
  useAppPath(appPath('', { agent: agent?.agentId }))
  const usable = useAgentBrowser(browserUsable)
  const operable = useAgentBrowser(canOperate)
  const loading = useAgentBrowser((snapshot) => snapshot.state?.loading ?? false)
  const desktop = useMediaQuery('(min-width: 769px)')
  const [roomy, setRoomy] = useState(true)
  const [sheet, setSheet] = useState<SheetName | null>(null)
  // Home is kept once it was shown, so what was being read is where it was after a look at the page or the conversation.
  const [homeSeen, setHomeSeen] = useState(mode === 'home')
  if (mode === 'home' && !homeSeen) setHomeSeen(true)
  const site = useSiteController()
  useEffect(() => {
    site.attach()
    return () => site.detach()
  }, [site])
  const stageRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const stage = stageRef.current
    if (!stage) return
    const observer = new ResizeObserver(() => {
      store.setStageSize({ width: stage.clientWidth, height: stage.clientHeight })
      setRoomy(stage.clientWidth >= ONE_ROW_MIN_WIDTH)
    })
    observer.observe(stage)
    return () => observer.disconnect()
  }, [store])
  const close = () => setSheet(null)
  const openTabs = () => setSheet('tabs')
  const oneRow = desktop && roomy
  const address = mode === 'chat' ? null : mode === 'home' ? <HomeAddressBar /> : <AddressBar onOpenTabs={oneRow ? undefined : openTabs} />
  return (
    <div className="ab-root" data-testid="agent-browser" data-mode={mode} data-agent={agent?.agentId} data-operable={operable ? 'true' : 'false'}>
      <div className="ab-top" data-layout={oneRow ? 'row' : 'stacked'} data-testid="ab-top">
        <div className="ab-identity" title={agent ? t('agentBrowser.agent.browserOf', 'This is {{name}}’s browser', { name: agent.name }) : undefined} data-testid="ab-identity">
          {agent ? <HubAvatar type="agent" name={agent.name} avatar={agent.avatar} size={24} /> : null}
          <span className="ab-identity-name" data-testid="ab-agent-name">{agent?.name}</span>
        </div>
        {oneRow ? address ?? <span className="ab-top-fill" aria-hidden /> : null}
        <div className="ab-modes" role="tablist" aria-label={t('agentBrowser.mode.label', 'View')}>
          {browserModes.map((item) => (
            <button key={item} type="button" role="tab" aria-selected={mode === item} className="ab-mode" data-testid={`ab-mode-${item}`} onClick={() => store.setMode(item)}>
              {t(`agentBrowser.mode.${item}`, item)}
            </button>
          ))}
        </div>
        {/* The tabs are the original page's; in the other modes their place stays, so the mode buttons do not move. */}
        {oneRow ? mode === 'page' ? <TabsButton onOpen={openTabs} /> : <span className="ab-tabs-slot" aria-hidden /> : null}
        <button type="button" className="ab-icon-button" aria-label={t('agentBrowser.menu.title', 'Browser settings')} aria-haspopup="dialog" data-testid="ab-menu-button" onClick={() => setSheet('menu')}><EllipsisVertical size={18} aria-hidden /></button>
      </div>
      {oneRow ? null : address}
      <div className="ab-progress" data-active={loading && mode === 'page' ? 'true' : 'false'} aria-hidden />
      <div ref={stageRef} className="ab-stage" data-testid="ab-stage" role="tabpanel">
        {homeSeen ? <HomePanel hidden={mode !== 'home'} /> : null}
        {mode === 'home' ? null : mode === 'chat' ? <ConversationPanel /> : usable ? <ScreenView /> : <Unavailable />}
        {usable && mode !== 'home' ? <RequestLayer /> : null}
        {mode === 'chat' ? null : <ConversationSheet />}
        <Toast />
      </div>
      <HandoverBar />
      {sheet === 'tabs' ? <TabsSheet onClose={close} /> : sheet === 'menu' ? <MenuSheet onClose={close} /> : null}
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
