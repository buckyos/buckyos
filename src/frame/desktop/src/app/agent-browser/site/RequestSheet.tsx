import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { displayUrl } from '../shell/address'
import { Sheet } from '../shell/Sheet'
import { useAgentBrowser } from '../store/context'
import { useSiteController } from './useSite'

/**
 * What the owner confirms before the Agent is asked about a page: adding its website to the personal site (`add`), or
 * tidying this one page (`tidy`) — which page, and one line the owner may add for the Agent.
 */
export function RequestSheet({ kind, url, title, onClose }: { kind: 'add' | 'tidy'; url: string; title: string; onClose: () => void }) {
  const site = useSiteController()
  const { t } = useI18n()
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  const [note, setNote] = useState('')
  let host = url
  try { host = new URL(url).hostname } catch { /* shown as it is */ }
  const confirm = () => {
    onClose()
    if (kind === 'add') site.requestAdd({ url, title, note })
    else site.tidyPage(url, title, note)
  }
  return (
    <Sheet
      testId={`ab-${kind}-sheet`}
      title={kind === 'add' ? t('agentBrowser.site.add.title', 'Add {{site}} to your reading', { site: host }) : t('agentBrowser.site.reflow.tidy', 'Tidy this page only')}
      onClose={onClose}
      footer={<>
        <button type="button" className="ab-button" data-testid={`ab-${kind}-cancel`} onClick={onClose}>{t('common.cancel')}</button>
        <button type="button" className="ab-button ab-button-primary" data-testid={`ab-${kind}-confirm`} onClick={confirm}>{kind === 'add' ? t('agentBrowser.site.add.confirm', 'Add') : t('agentBrowser.site.tidy.confirm', 'Tidy it')}</button>
      </>}
    >
      <p className="ab-dialog-message">
        {kind === 'add'
          ? t('agentBrowser.site.add.body', '{{name}} collects this website into your reading site and keeps it up to date. Getting it ready takes a few minutes; you can go on reading meanwhile.', { name })
          : t('agentBrowser.site.tidy.body', '{{name}} makes a version of this one page that is easy to read on your phone. It shows in Home when it is done; the website is not added to your reading.', { name })}
      </p>
      <p className="ab-page-ref" data-testid={`ab-${kind}-page`}>
        {title ? <span className="ab-page-ref-title">{title}</span> : null}
        <span className="ab-page-ref-url">{displayUrl(url)}</span>
      </p>
      <label className="ab-field-label">
        {t('agentBrowser.site.add.note', 'Anything {{name}} should know? (optional)', { name })}
        <input
          className="ab-field"
          data-testid={`ab-${kind}-note`}
          type="text"
          maxLength={200}
          enterKeyHint="done"
          value={note}
          onChange={(event) => setNote(event.target.value)}
          onKeyDown={(event) => { if (event.key === 'Enter') confirm() }}
        />
      </label>
    </Sheet>
  )
}
