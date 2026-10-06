/* The freshness of a generated result or wish, in words (phase two §7.5): the same badge on the
 * wish Block, the result Blocks and in the data-source view, all reading what the core computed. */

import { useFreshness } from '../../state/hooks'
import { FRESHNESS_LABEL, freshnessDetail } from './freshnessText'

export function FreshnessBadge({ entityId, showManual = true }: { entityId: string; showManual?: boolean }) {
  const info = useFreshness(entityId)
  if (!info || info.status === 'none') return null
  const tone = info.status === 'current' ? 'aiws-chip-ok' : info.status === 'unknown' ? '' : 'aiws-chip-warn'
  return (
    <span className="aiws-freshness" data-testid={`aiws-freshness-${entityId}`} data-status={info.status} data-manual={info.manual_modified ? 'true' : 'false'}>
      <span className={`aiws-chip ${tone}`} title={freshnessDetail(info)}>{FRESHNESS_LABEL[info.status]}</span>
      {showManual && info.manual_modified && <span className="aiws-chip aiws-chip-warn" title="结果的当前内容版本不等于生成版本，且变化不来自许愿格的后续运行">人工已修改</span>}
      {info.simulated && <span className="aiws-chip aiws-chip-derived">模拟</span>}
    </span>
  )
}
