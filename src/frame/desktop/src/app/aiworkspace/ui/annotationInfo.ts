/* Annotation wording shared by the panel and the editors (design §3.7): what an anchor's state means,
 * what an annotation is about, jumping to where it is shown. */

import type { AnchorInfo } from '../api/types'
import type { AnnotationMark } from '../state/hooks'
import { describeTarget } from './creators'

/** The anchor state as the backend resolved it, in words (`aiws-anchor-state`). */
export function anchorStatus(anchor: AnchorInfo): string {
  switch (anchor.state) {
    case 'resolved':
      if (anchor.range_status === 'relocated') return '已按原文重新定位'
      if (anchor.range_status === 'unchecked') return '锚点有效（由应用定位）'
      return '锚点有效'
    case 'degraded':
      if (anchor.level === 'range') return '原位置已删除，已按原文重新定位'
      if (anchor.level === 'target' && anchor.position) return '原块已删除，已按原文找到所在块'
      if (anchor.level === 'target') return '精确位置已失效，显示在所在块'
      return '原位置已删除，显示在对象上'
    case 'target_deleted':
      return '目标已删除'
    default:
      return '当前版本无法定位，显示在对象上'
  }
}

export function annotationLabel(mark: AnnotationMark): string {
  return mark.payload.context?.label ?? describeTarget(mark.payload.target)
}

/** Scroll to where the annotation is shown, if it is shown. */
export function revealAnnotation(entityId: string) {
  const element = document.querySelector(`[data-anno-${CSS.escape(entityId)}]`)
  element?.scrollIntoView({ block: 'center', behavior: 'smooth' })
  return element !== null
}
