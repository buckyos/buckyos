/* Annotation wording shared by the panel and the editors (design §3.7): what an anchor's state means,
 * what an annotation is about, jumping to where it is shown. */

import type { AnchorInfo } from '../api/types'
import { aiwsT } from '../i18n'
import type { AnnotationMark } from '../state/hooks'
import { describeTarget } from './creators'

/** The anchor state as the backend resolved it, in words (`aiws-anchor-state`). */
export function anchorStatus(anchor: AnchorInfo): string {
  switch (anchor.state) {
    case 'resolved':
      if (anchor.range_status === 'relocated') return aiwsT('aiworkspace.data.anchor.relocated')
      if (anchor.range_status === 'unchecked') return aiwsT('aiworkspace.data.anchor.validByApp')
      return aiwsT('aiworkspace.data.anchor.valid')
    case 'degraded':
      if (anchor.level === 'range') return aiwsT('aiworkspace.data.anchor.rangeDeleted')
      if (anchor.level === 'target' && anchor.position) return aiwsT('aiworkspace.data.anchor.blockDeleted')
      if (anchor.level === 'target') return aiwsT('aiworkspace.data.anchor.exactLostOnBlock')
      return aiwsT('aiworkspace.data.anchor.deletedOnObject')
    case 'target_deleted':
      return aiwsT('aiworkspace.data.anchor.targetDeleted')
    default:
      return aiwsT('aiworkspace.data.anchor.unresolved')
  }
}

export function annotationLabel(mark: AnnotationMark): string {
  return mark.payload.context?.label ?? (mark.payload.target ? describeTarget(mark.payload.target) : aiwsT('aiworkspace.data.stickyNote'))
}

/** Scroll to where the annotation is shown, if it is shown. */
export function revealAnnotation(entityId: string) {
  const element = document.querySelector(`[data-anno-${CSS.escape(entityId)}]`)
  element?.scrollIntoView({ block: 'center', behavior: 'smooth' })
  return element !== null
}
