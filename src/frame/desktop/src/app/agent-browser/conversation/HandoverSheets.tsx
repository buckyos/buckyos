/* What the status strip asks before the page changes hands: with "Hand to Agent" the owner may add what the Agent
 * should do (DG2); with "Hand back" the message that tells the Agent to go on can be changed (DG3). */

import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { displayUrl } from '../shell/address'
import { Sheet } from '../shell/Sheet'
import { useAgentBrowser, useAgentBrowserStore } from '../store/context'
import './conversation.css'

export function HandToAgentSheet({ onClose }: { onClose: () => void }) {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  const tab = useAgentBrowser((snapshot) => snapshot.group?.tabs.find((item) => item.active))
  const [note, setNote] = useState('')
  const url = tab && tab.url !== 'about:blank' ? tab.url : ''
  const label = t('agentBrowser.handToAgent.note', 'What should it do? (optional)')
  return (
    <Sheet
      testId="ab-hand-to-agent-sheet"
      title={t('agentBrowser.handToAgent.title', 'Hand to {{name}}', { name })}
      onClose={onClose}
      footer={<>
        <button type="button" className="ab-button" data-testid="ab-handover-cancel" onClick={onClose}>{t('common.cancel')}</button>
        <button type="button" className="ab-button ab-button-primary" data-testid="ab-handover-confirm" onClick={() => { onClose(); void store.handToAgent(note) }}>{t('agentBrowser.handToAgent.confirm', 'Hand over')}</button>
      </>}
    >
      <p className="ab-dialog-message">{url ? t('agentBrowser.handToAgent.body', '{{name}} continues from the page you are on.', { name }) : t('agentBrowser.handToAgent.noPage', 'No page is open yet. Tell {{name}} what to do.', { name })}</p>
      {url ? (
        <p className="ab-page-ref" data-testid="ab-handover-page">
          {tab?.title ? <span className="ab-page-ref-title">{tab.title}</span> : null}
          <span className="ab-page-ref-url">{displayUrl(url)}</span>
        </p>
      ) : null}
      <textarea className="ab-note-field" data-testid="ab-handover-note" aria-label={label} placeholder={label} value={note} onChange={(event) => setNote(event.target.value)} />
    </Sheet>
  )
}

export function HandBackSheet({ onClose }: { onClose: () => void }) {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  const requested = useAgentBrowser((snapshot) => !!snapshot.group?.handover)
  const [text, setText] = useState(() => (requested ? t('agentBrowser.handBack.default', 'Done, please continue.') : t('agentBrowser.handBack.defaultOwn', 'I am done here, please continue.')))
  const label = t('agentBrowser.handBack.message', 'Message to {{name}}', { name })
  return (
    <Sheet
      testId="ab-hand-back-sheet"
      title={t('agentBrowser.handBack.title', 'Hand back to {{name}}', { name })}
      onClose={onClose}
      footer={<>
        <button type="button" className="ab-button" data-testid="ab-handover-cancel" onClick={onClose}>{t('common.cancel')}</button>
        <button type="button" className="ab-button ab-button-primary" data-testid="ab-handover-confirm" onClick={() => { onClose(); void store.handBack(text) }}>{t('agentBrowser.action.handBack', 'Hand back')}</button>
      </>}
    >
      <p className="ab-dialog-message">{t('agentBrowser.handBack.body', '{{name}} gets the page back together with this message, and goes on.', { name })}</p>
      <label className="ab-note-label" htmlFor="ab-handover-note">{label}</label>
      <textarea id="ab-handover-note" className="ab-note-field" data-testid="ab-handover-note" value={text} onChange={(event) => setText(event.target.value)} />
      {text.trim() ? null : <p className="ab-note-hint" role="note" data-testid="ab-handover-empty">{t('agentBrowser.handBack.empty', 'Without a message {{name}} is not told to go on.', { name })}</p>}
    </Sheet>
  )
}
