/* Freshness states in words (phase two §7.5). */

import type { FreshnessInfo } from '../../api/types'

export const FRESHNESS_LABEL: Record<FreshnessInfo['status'], string> = {
  current: '当前有效', stale: '需要刷新', upstream_stale: '上游需要刷新', unavailable: '引用不可用', unknown: '无法确认', none: '',
}

export function freshnessDetail(info: FreshnessInfo): string {
  if (info.status === 'stale') {
    const names = (info.changed_inputs ?? []).map((line) => line.name ?? line.label ?? line.entity_id)
    return `此结果生成后，${names.length ? names.join(' / ') : '输入'} 已改变`
  }
  if (info.status === 'upstream_stale') return `直接输入未变，但 ${(info.upstream ?? []).map((u) => u.entity_id).join(' / ')} 本身已过期`
  if (info.status === 'unavailable') return `输入不可用：${(info.inputs ?? []).filter((l) => l.reason).map((l) => `${l.name ?? l.entity_id}（${l.reason === 'deleted' ? '已删除' : l.reason === 'missing' ? '不存在' : l.reason === 'selector_lost' ? '选择范围失效' : l.reason}）`).join('、')}`
  if (info.status === 'unknown') return info.error ? `无法确认：${info.error.detail ?? info.error.code}` : '无法确认：输入不可读或没有可比较的版本'
  if (info.imported_stale) return '导入时已过期，运行后恢复'
  return '所有需要跟随当前数据的输入与记录一致'
}
