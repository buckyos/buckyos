/* The cards that show annotations next to the content of an editor (design §3.7). */

import { useLayoutEffect, useRef, useState } from 'react'
import type { Placement } from '../anchors/richtext'
import type { AnnotationMark } from '../state/hooks'
import { authorLabel } from '../state/guest'
import { useAiwsText, type AiwsUiText } from '../useAiwsText'

/** Where an editor placed it, when that is coarser or elsewhere than recorded. */
function placementHint(t: AiwsUiText['t'], mark: AnnotationMark, placement: Placement | undefined): string | null {
  if (!placement || placement.level === 'entity') return mark.payload.target.selector ? t('aiworkspace.data.anchor.positionGone') : null
  if (placement.relocated) return t('aiworkspace.data.anchor.relocated')
  if (mark.payload.range && placement.level === 'target') return t('aiworkspace.data.anchor.exactLost')
  return null
}

/** Cards next to the content, each level with its anchor; stacked below it when the editor is narrow. */
export function AnnotationGutter({ container, marks, placements, active, onActivate, version }: {
  container: HTMLElement | null
  marks: AnnotationMark[]
  placements: Placement[]
  active: string | null
  onActivate: (entityId: string | null) => void
  /** Changes whenever the placed content may have moved. */
  version: number
}) {
  const { t } = useAiwsText()
  const ref = useRef<HTMLDivElement>(null)
  const [narrow, setNarrow] = useState(false)
  const byId = new Map(marks.map((mark) => [mark.entityId, mark]))
  const ordered = placements.filter((placement) => byId.has(placement.id))

  useLayoutEffect(() => {
    const gutter = ref.current
    if (!gutter || !container) return
    const layout = () => {
      const wide = (gutter.parentElement?.clientWidth ?? 0) >= 640
      setNarrow(!wide)
      if (!wide) { gutter.style.minHeight = ''; return }
      const base = gutter.getBoundingClientRect().top
      let bottom = 0
      gutter.querySelectorAll<HTMLElement>('[data-anno-card]').forEach((card) => {
        const id = card.dataset.annoCard ?? ''
        const anchor = container.querySelector(`[data-anno-${CSS.escape(id)}]`)
        const top = Math.max(anchor ? anchor.getBoundingClientRect().top - base : 0, bottom)
        card.style.top = `${top}px`
        bottom = top + card.offsetHeight + 6
      })
      gutter.style.minHeight = `${bottom}px`
    }
    layout()
    // the next frame: laying out resizes what is observed
    let frame = 0
    const observer = new ResizeObserver(() => { cancelAnimationFrame(frame); frame = requestAnimationFrame(layout) })
    observer.observe(container)
    if (gutter.parentElement) observer.observe(gutter.parentElement)
    return () => { cancelAnimationFrame(frame); observer.disconnect() }
  }, [container, placements, version, marks, active, narrow])

  if (ordered.length === 0) return null
  return (
    <div ref={ref} className={`aiws-anno-gutter${narrow ? ' is-narrow' : ''}`} data-testid="aiws-anno-gutter">
      {ordered.map((placement) => {
        const mark = byId.get(placement.id) as AnnotationMark
        const hint = placementHint(t, mark, placement)
        return (
          <button
            key={placement.id}
            type="button"
            className={`aiws-anno-card${placement.id === active ? ' is-active' : ''}`}
            data-anno-card={placement.id}
            data-level={placement.level}
            data-degraded={hint ? 'true' : undefined}
            data-testid="aiws-anno-card"
            onClick={() => onActivate(placement.id === active ? null : placement.id)}
          >
            <span className="aiws-anno-card-body">{mark.payload.body}</span>
            <span className="aiws-anno-card-meta">
              {authorLabel(mark.payload) && <span>{authorLabel(mark.payload)}</span>}
              {hint && <span data-testid="aiws-anno-card-hint">{hint}</span>}
            </span>
          </button>
        )
      })}
    </div>
  )
}
