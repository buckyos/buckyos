import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { Sheet } from '../shell/Sheet'
import { useAgentBrowser } from '../store/context'
import { useReflow, useReflowController } from './useReflow'

/** The user's preference for the site of the page: one sentence in their own words, which the Agent makes the site's pages by. */
export function PreferenceSheet({ onClose }: { onClose: () => void }) {
  const controller = useReflowController()
  const { t } = useI18n()
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  const preference = useReflow((snapshot) => snapshot.preference)
  const address = useReflow((snapshot) => snapshot.state?.url ?? '')
  const [text, setText] = useState(preference)
  const [saving, setSaving] = useState(false)
  const host = address ? new URL(address).hostname : ''
  const save = async () => {
    setSaving(true)
    const saved = await controller.savePreference(text)
    setSaving(false)
    if (saved) onClose()
  }
  return (
    <Sheet
      testId="ab-reflow-preference-sheet"
      title={t('agentBrowser.reflow.preference.title', 'Preference for {{host}}', { host })}
      onClose={onClose}
      footer={<>
        <button type="button" className="ab-button" data-testid="ab-reflow-preference-cancel" onClick={onClose}>{t('common.cancel')}</button>
        <button type="button" className="ab-button ab-button-primary" disabled={saving} data-testid="ab-reflow-preference-save" onClick={() => void save()}>{t('common.save')}</button>
      </>}
    >
      <p className="ab-dialog-message">{t('agentBrowser.reflow.preference.hint', 'Say in a sentence how you want the pages of this site shown. {{name}} makes them by it, and is told when you change it.', { name })}</p>
      <textarea
        className="ab-reflow-textarea"
        data-testid="ab-reflow-preference-input"
        rows={3}
        maxLength={500}
        aria-label={t('agentBrowser.reflow.preference.label', 'Preference')}
        placeholder={t('agentBrowser.reflow.preference.placeholder', 'For example: title, date and text only, one column')}
        value={text}
        onChange={(event) => setText(event.target.value)}
      />
    </Sheet>
  )
}
