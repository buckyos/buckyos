import { ArrowLeft, ArrowRight, Layers, RotateCw, X } from 'lucide-react'
import { useRef, useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { useAgentBrowser, useAgentBrowserStore } from '../store/context'
import { canOperate } from '../store/store'
import { displayUrl } from './address'

/** The tabs of the browser, with how many there are: at the end of the address bar on a phone, beside the menu on a desktop. */
export function TabsButton({ onOpen }: { onOpen: () => void }) {
  const { t } = useI18n()
  const tabCount = useAgentBrowser((snapshot) => snapshot.group?.tabs.length ?? 0)
  return (
    <button type="button" className="ab-icon-button ab-tabs-button" aria-label={t('agentBrowser.tabs.open', 'Tabs: {{count}}', { count: tabCount })} data-testid="ab-tabs-button" onClick={onOpen}>
      <Layers size={18} aria-hidden />
      <span className="ab-tabs-count" data-testid="ab-tabs-count">{tabCount}</span>
    </button>
  )
}

/** Back, forward (when there is somewhere to go), the address with reload / stop, and — given `onOpenTabs` — the tabs. */
export function AddressBar({ onOpenTabs }: { onOpenTabs?: () => void }) {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const state = useAgentBrowser((snapshot) => snapshot.state)
  const operable = useAgentBrowser(canOperate)
  const inputRef = useRef<HTMLInputElement>(null)
  const [draft, setDraft] = useState<string | null>(null)
  const url = state?.url ?? ''
  const loading = state?.loading ?? false

  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    if (draft !== null && store.navigate(draft)) inputRef.current?.blur()
  }

  return (
    <div className="ab-address" data-testid="ab-address" data-loading={loading ? 'true' : undefined}>
      <button type="button" className="ab-icon-button" aria-label={t('agentBrowser.nav.back', 'Back')} disabled={!operable || !state?.can_go_back} data-testid="ab-back" onClick={() => store.nav('back')}><ArrowLeft size={18} aria-hidden /></button>
      {state?.can_go_forward ? <button type="button" className="ab-icon-button" aria-label={t('agentBrowser.nav.forward', 'Forward')} disabled={!operable} data-testid="ab-forward" onClick={() => store.nav('forward')}><ArrowRight size={18} aria-hidden /></button> : null}
      <form className="ab-address-field" onSubmit={submit}>
        <input
          ref={inputRef}
          className="ab-address-input"
          data-testid="ab-address-input"
          type="text"
          inputMode="url"
          enterKeyHint="go"
          autoCapitalize="off"
          autoCorrect="off"
          autoComplete="off"
          spellCheck={false}
          aria-label={t('agentBrowser.address.label', 'Address')}
          placeholder={t('agentBrowser.address.placeholder', 'Search or enter an address')}
          readOnly={!operable}
          value={draft ?? displayUrl(url)}
          onFocus={(event) => {
            if (!operable) return
            setDraft(url)
            const input = event.currentTarget
            window.requestAnimationFrame(() => { if (document.activeElement === input) input.select() })
          }}
          onBlur={() => setDraft(null)}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => { if (event.key === 'Escape') { event.stopPropagation(); event.currentTarget.blur() } }}
        />
        {state?.active_tab_id ? (
          <button
            type="button"
            className="ab-icon-button ab-address-reload"
            aria-label={loading ? t('agentBrowser.nav.stop', 'Stop') : t('agentBrowser.nav.reload', 'Reload')}
            disabled={!operable}
            data-testid="ab-reload"
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => store.nav(loading ? 'stop' : 'reload')}
          >
            {loading ? <X size={16} aria-hidden /> : <RotateCw size={16} aria-hidden />}
          </button>
        ) : null}
      </form>
      {onOpenTabs ? <TabsButton onOpen={onOpenTabs} /> : null}
    </div>
  )
}
