import { ArrowLeft, RotateCw } from 'lucide-react'
import { useRef, useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { displayUrl, toNavigableUrl } from '../shell/address'
import { currentSource } from './controller'
import { useSite, useSiteController } from './useSite'

/**
 * The address bar of home mode. It shows where what is being read comes from (the article's or the source's own
 * address; nothing at the site's front page), goes back through the pages read here, and reads the page again. An
 * address entered here shows what the owner has for it — its page of the site, the page tidied for it — and is the
 * browser's otherwise: original page mode opens it.
 */
export function HomeAddressBar() {
  const site = useSiteController()
  const { t } = useI18n()
  const place = useSite((snapshot) => snapshot.place)
  const canGoBack = useSite((snapshot) => snapshot.canGoBack)
  const article = useSite((snapshot) => snapshot.article)
  const source = useSite(currentSource)
  const inputRef = useRef<HTMLInputElement>(null)
  const [draft, setDraft] = useState<string | null>(null)
  const tidied = place.kind === 'page'
  const url = tidied ? place.url : article?.url ?? source?.home_url ?? ''

  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    const target = draft === null ? null : toNavigableUrl(draft)
    if (!target) return
    inputRef.current?.blur()
    void site.openUrl(target)
  }

  return (
    <div className="ab-address" data-testid="ab-address" data-home="true">
      <button type="button" className="ab-icon-button" aria-label={t('agentBrowser.nav.back', 'Back')} disabled={!canGoBack} data-testid="ab-back" onClick={() => site.back()}><ArrowLeft size={18} aria-hidden /></button>
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
          value={draft ?? displayUrl(url)}
          onFocus={(event) => {
            setDraft(url)
            const input = event.currentTarget
            window.requestAnimationFrame(() => { if (document.activeElement === input) input.select() })
          }}
          onBlur={() => setDraft(null)}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => { if (event.key === 'Escape') { event.stopPropagation(); event.currentTarget.blur() } }}
        />
        <button
          type="button"
          className="ab-icon-button ab-address-reload"
          aria-label={tidied ? t('agentBrowser.reflow.refresh', 'Make this page again') : t('agentBrowser.nav.reload', 'Reload')}
          data-testid={tidied ? 'ab-reflow-refresh' : 'ab-home-reload'}
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => site.reload()}
        >
          <RotateCw size={16} aria-hidden />
        </button>
      </form>
    </div>
  )
}
