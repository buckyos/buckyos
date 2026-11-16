/* The picture setting of the browser's menu: automatic (the screen follows what the connection carries), clear, or
 * data saver; in automatic it says which tier the screen is at now. */

import { MonitorSmartphone } from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { useAgentBrowser, useAgentBrowserStore } from '../store/context'
import { QUALITY_CHOICES } from './quality'
import './quality.css'

export function QualityItem() {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const choice = useAgentBrowser((snapshot) => snapshot.screenQuality)
  const tier = useAgentBrowser((snapshot) => snapshot.screenTier)
  const detail = choice === 'auto' && tier
    ? t('agentBrowser.quality.now', 'Follows your connection · now {{tier}}', { tier: t(`agentBrowser.quality.tier.${tier}`, tier) })
    : t(`agentBrowser.quality.hint.${choice}`, '')
  return (
    <div className="ab-list-item ab-quality" data-testid="ab-quality">
      <MonitorSmartphone size={18} aria-hidden />
      <span className="ab-list-text">
        <span className="ab-list-title">{t('agentBrowser.quality.title', 'Picture')}</span>
        <span className="ab-list-detail" data-testid="ab-quality-detail">{detail}</span>
        <span className="ab-quality-choices" role="radiogroup" aria-label={t('agentBrowser.quality.title', 'Picture')}>
          {QUALITY_CHOICES.map((value) => (
            <button key={value} type="button" role="radio" aria-checked={value === choice} className="ab-quality-choice" data-testid={`ab-quality-${value}`} onClick={() => store.setScreenQuality(value)}>
              {t(`agentBrowser.quality.${value}`, value)}
            </button>
          ))}
        </span>
      </span>
    </div>
  )
}
