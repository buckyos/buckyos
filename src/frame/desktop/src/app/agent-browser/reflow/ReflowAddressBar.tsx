import { ArrowLeft, RotateCw } from 'lucide-react'
import { useRef, useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { displayUrl } from '../shell/address'
import { useReflow, useReflowController } from './useReflow'

/**
 * The address bar of reflow mode: the address of the original page (the page shown is made from it), back to the
 * page read before, and making the page again. Nothing here goes to the browser: the screen is closed in this mode.
 */
export function ReflowAddressBar() {
  const controller = useReflowController()
  const { t } = useI18n()
  const url = useReflow((snapshot) => snapshot.url)
  const canGoBack = useReflow((snapshot) => snapshot.canGoBack)
  const job = useReflow((snapshot) => snapshot.job)
  const known = useReflow((snapshot) => snapshot.state !== null)
  const inputRef = useRef<HTMLInputElement>(null)
  const [draft, setDraft] = useState<string | null>(null)

  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    if (draft !== null && controller.go(draft)) inputRef.current?.blur()
  }

  return (
    <div className="ab-address" data-testid="ab-address" data-reflow="true">
      <button type="button" className="ab-icon-button" aria-label={t('agentBrowser.nav.back', 'Back')} disabled={!canGoBack} data-testid="ab-back" onClick={() => controller.back()}><ArrowLeft size={18} aria-hidden /></button>
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
        {known ? (
          <button
            type="button"
            className="ab-icon-button ab-address-reload"
            aria-label={t('agentBrowser.reflow.refresh', 'Make this page again')}
            disabled={job !== null}
            data-testid="ab-reflow-refresh"
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => void controller.refresh()}
          >
            <RotateCw size={16} aria-hidden className={job ? 'ab-spin' : undefined} />
          </button>
        ) : null}
      </form>
    </div>
  )
}
