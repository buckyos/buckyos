/* Freshness states in words (phase two §7.5). */

import type { FreshnessInfo } from '../../api/types'
import { aiwsT, aiwsText } from '../../i18n'

export const FRESHNESS_LABEL: Readonly<Record<FreshnessInfo['status'], string>> = {
  get current() { return aiwsT('aiworkspace.data.freshness.current') },
  get stale() { return aiwsT('aiworkspace.data.freshness.stale') },
  get upstream_stale() { return aiwsT('aiworkspace.data.freshness.upstreamStale') },
  get unavailable() { return aiwsT('aiworkspace.data.freshness.unavailable') },
  get unknown() { return aiwsT('aiworkspace.data.freshness.unknown') },
  none: '',
}

export function freshnessDetail(info: FreshnessInfo): string {
  if (info.status === 'stale') {
    const names = (info.changed_inputs ?? []).map((line) => line.name ?? line.label ?? line.entity_id)
    return names.length ? aiwsT('aiworkspace.data.freshness.staleNamed', { names: names.join(' / ') }) : aiwsT('aiworkspace.data.freshness.staleInputs')
  }
  if (info.status === 'upstream_stale') return aiwsT('aiworkspace.data.freshness.upstreamDetail', { names: (info.upstream ?? []).map((u) => u.entity_id).join(' / ') })
  if (info.status === 'unavailable') {
    const items = (info.inputs ?? []).filter((l) => l.reason).map((l) => aiwsT('aiworkspace.data.freshness.inputItem', {
      name: l.name ?? l.entity_id,
      reason: l.reason === 'deleted' ? aiwsT('aiworkspace.data.freshness.reasonDeleted') : l.reason === 'missing' ? aiwsT('aiworkspace.data.freshness.reasonMissing')
        : l.reason === 'selector_lost' ? aiwsT('aiworkspace.data.freshness.reasonSelectorLost') : String(l.reason),
    }))
    return aiwsT('aiworkspace.data.freshness.unavailableDetail', { items: aiwsText().list(items) })
  }
  if (info.status === 'unknown') return info.error ? aiwsT('aiworkspace.data.freshness.unknownError', { error: info.error.detail ?? info.error.code }) : aiwsT('aiworkspace.data.freshness.unknownDetail')
  if (info.imported_stale) return aiwsT('aiworkspace.data.freshness.importedStale')
  return aiwsT('aiworkspace.data.freshness.currentDetail')
}
