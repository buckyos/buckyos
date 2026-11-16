/* The App's own panels: the tabs of the browser, and the settings. */

import { LayoutList, Plus, Trash2, Users, X } from 'lucide-react'
import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { ReflowLibrarySheet } from '../reflow/LibrarySheet'
import { QualityItem } from '../screen/QualityItem'
import { useAgentBrowser, useAgentBrowserStore } from '../store/context'
import { canOperate } from '../store/store'
import { displayUrl } from './address'
import { Sheet } from './Sheet'

/** The tabs of the browser: switch, close, open a new one. */
export function TabsSheet({ onClose }: { onClose: () => void }) {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const tabs = useAgentBrowser((snapshot) => snapshot.group?.tabs)
  const operable = useAgentBrowser(canOperate)
  return (
    <Sheet
      testId="ab-tabs"
      title={t('agentBrowser.tabs.title', 'Tabs')}
      onClose={onClose}
      footer={<button type="button" className="ab-button ab-button-primary" disabled={!operable} data-testid="ab-tab-new" onClick={() => { store.openTab(); onClose() }}><Plus size={16} aria-hidden />{t('agentBrowser.tabs.new', 'New tab')}</button>}
    >
      {tabs && tabs.length > 0 ? (
        <ul className="ab-list" data-testid="ab-tab-list">
          {tabs.map((tab) => (
            <li key={tab.tab_id} className="ab-tab-row" data-testid="ab-tab" data-active={tab.active ? 'true' : 'false'}>
              <button type="button" className="ab-list-item" aria-current={tab.active ? 'true' : undefined} disabled={!operable && !tab.active} data-testid={`ab-tab-${tab.tab_id}`} onClick={() => { if (!tab.active) store.activateTab(tab.tab_id); onClose() }}>
                <span className="ab-list-text">
                  <span className="ab-list-title">{tab.title || t('agentBrowser.tabs.blank', 'New tab')}</span>
                  <span className="ab-list-detail">{tab.loading ? t('agentBrowser.tabs.loading', 'Loading…') : displayUrl(tab.url)}</span>
                </span>
              </button>
              <button type="button" className="ab-icon-button" aria-label={t('agentBrowser.tabs.close', 'Close tab')} disabled={!operable} data-testid={`ab-tab-close-${tab.tab_id}`} onClick={() => store.closeTab(tab.tab_id)}><X size={16} aria-hidden /></button>
            </li>
          ))}
        </ul>
      ) : <p className="ab-sheet-empty" data-testid="ab-tabs-empty">{t('agentBrowser.tabs.empty', 'No tabs yet.')}</p>}
    </Sheet>
  )
}

/** Settings: the desktop layout, and clearing the browser's sign-ins (with a confirmation). */
export function MenuSheet({ onClose }: { onClose: () => void }) {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const agent = useAgentBrowser((snapshot) => snapshot.agent)
  const agentCount = useAgentBrowser((snapshot) => snapshot.agents.length)
  const desktopSite = useAgentBrowser((snapshot) => snapshot.desktopSite)
  const version = useAgentBrowser((snapshot) => snapshot.browser?.version)
  const pending = useAgentBrowser((snapshot) => snapshot.pending)
  const [confirming, setConfirming] = useState(false)
  const [library, setLibrary] = useState(false)
  if (library) return <ReflowLibrarySheet onClose={() => setLibrary(false)} />
  if (confirming) {
    return (
      <Sheet
        testId="ab-clear-confirm"
        title={t('agentBrowser.clear.title', 'Clear sign-ins?')}
        onClose={() => setConfirming(false)}
        footer={<>
          <button type="button" className="ab-button" data-testid="ab-clear-cancel" onClick={() => setConfirming(false)}>{t('common.cancel')}</button>
          <button type="button" className="ab-button ab-button-danger" disabled={pending !== null} data-testid="ab-clear-accept" onClick={() => { void store.clearProfile().then((cleared) => { if (cleared) onClose() }) }}>{t('agentBrowser.clear.accept', 'Clear')}</button>
        </>}
      >
        <p className="ab-dialog-message">{t('agentBrowser.clear.body', '{{name}}’s browser will be signed out of every website, and its cookies and site data will be deleted. You will have to sign in again before {{name}} can use those websites.', { name: agent?.name ?? '' })}</p>
      </Sheet>
    )
  }
  return (
    <Sheet testId="ab-menu" title={t('agentBrowser.menu.title', 'Browser settings')} onClose={onClose}>
      <ul className="ab-list">
        <li>
          <button type="button" role="switch" aria-checked={desktopSite} className="ab-list-item" data-testid="ab-desktop-site" onClick={() => store.setDesktopSite(!desktopSite)}>
            <span className="ab-list-text">
              <span className="ab-list-title">{t('agentBrowser.menu.desktopSite', 'Desktop site')}</span>
              <span className="ab-list-detail">{t('agentBrowser.menu.desktopSiteHint', 'Show pages in their desktop layout, scaled to fit. Pinch to zoom.')}</span>
            </span>
            <span className="ab-switch" aria-hidden />
          </button>
        </li>
        <li><QualityItem /></li>
        <li>
          <button type="button" className="ab-list-item" data-testid="ab-reflow-library-open" onClick={() => setLibrary(true)}>
            <LayoutList size={18} aria-hidden />
            <span className="ab-list-text">
              <span className="ab-list-title">{t('agentBrowser.menu.reflowLibrary', 'Reflow library')}</span>
              <span className="ab-list-detail">{t('agentBrowser.menu.reflowLibraryHint', 'The sites that were reflowed for you: look inside, delete, import a package.')}</span>
            </span>
          </button>
        </li>
        <li>
          <button type="button" className="ab-list-item" data-testid="ab-clear-profile" onClick={() => setConfirming(true)}>
            <Trash2 size={18} aria-hidden />
            <span className="ab-list-text">
              <span className="ab-list-title">{t('agentBrowser.menu.clear', 'Clear sign-ins')}</span>
              <span className="ab-list-detail">{t('agentBrowser.menu.clearHint', 'Sign this browser out of every website.')}</span>
            </span>
          </button>
        </li>
        {agentCount > 1 ? (
          <li>
            <button type="button" className="ab-list-item" data-testid="ab-menu-switch-agent" onClick={() => { onClose(); store.leaveAgent() }}>
              <Users size={18} aria-hidden />
              <span className="ab-list-text"><span className="ab-list-title">{t('agentBrowser.agent.switchLong', 'Use another Agent’s browser')}</span></span>
            </button>
          </li>
        ) : null}
      </ul>
      {version ? <p className="ab-sheet-meta" data-testid="ab-version">Chromium {version.browser} · Playwright {version.playwright}</p> : null}
    </Sheet>
  )
}
