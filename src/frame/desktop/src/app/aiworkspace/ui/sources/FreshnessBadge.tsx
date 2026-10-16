/* The freshness of a generated result or wish, in words (phase two §7.5): the same badge on the
 * wish Block, the result Blocks and in the data-source view, all reading what the core computed. */

import { useFreshness } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { FRESHNESS_LABEL, freshnessDetail } from './freshnessText'

export function FreshnessBadge({ entityId, showManual = true }: { entityId: string; showManual?: boolean }) {
  const { t } = useAiwsText()
  const info = useFreshness(entityId)
  if (!info || info.status === 'none') return null
  const tone = info.status === 'current' ? 'aiws-chip-ok' : info.status === 'unknown' ? '' : 'aiws-chip-warn'
  return (
    <span className="aiws-freshness" data-testid={`aiws-freshness-${entityId}`} data-status={info.status} data-manual={info.manual_modified ? 'true' : 'false'}>
      <span className={`aiws-chip ${tone}`} title={freshnessDetail(info)}>{FRESHNESS_LABEL[info.status]}</span>
      {showManual && info.manual_modified && <span className="aiws-chip aiws-chip-warn" title={t('aiworkspace.data.freshness.manualTitle')}>{t('aiworkspace.data.freshness.manual')}</span>}
      {info.simulated && <span className="aiws-chip aiws-chip-derived">{t('aiworkspace.data.simulated')}</span>}
      {info.external_data && <span className="aiws-chip" title={t('aiworkspace.data.freshness.externalTitle')}>{t('aiworkspace.data.freshness.external')}</span>}
      {info.model_judgment && <span className="aiws-chip" title={t('aiworkspace.data.freshness.modelTitle')}>{t('aiworkspace.data.freshness.model')}</span>}
    </span>
  )
}
