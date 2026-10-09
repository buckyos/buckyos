import { ChevronDown, ChevronUp, X } from 'lucide-react'
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { useI18n } from '../../../i18n/provider'
import type { SessionPanelInfo } from '../types'

const toneColor = { success: 'var(--cp-success)', warning: 'var(--cp-warning)', danger: 'var(--cp-danger)' } as const

/**
 * The optional panel floating over the top of a conversation: a pinned
 * message, or host-defined content such as the state of a ticket. Collapsed it
 * is at most three text lines high (title, fields and text share them);
 * expanding shows the whole text, every field and the custom `children`.
 */
export function SessionPanel({ icon, title, text, fields, children, dismissLabel, onDismiss, onCollapsedHeight, testId = 'session-panel' }: {
  icon?: ReactNode
  title: string
  text?: string
  fields?: SessionPanelInfo['fields']
  /** Extra content shown only while expanded. */
  children?: ReactNode
  dismissLabel?: string
  onDismiss?: () => void
  /** Height the collapsed panel takes from the top of the history, so the first rows stay readable. */
  onCollapsedHeight?: (height: number) => void
  testId?: string
}) {
  const { t } = useI18n()
  const [expanded, setExpanded] = useState(false)
  const [overflows, setOverflows] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  const textRef = useRef<HTMLParagraphElement>(null)
  const fieldsRef = useRef<HTMLDListElement>(null)
  const hasFields = !!fields?.length
  const textLines = Math.max(1, 3 - 1 - (hasFields ? 1 : 0))

  useLayoutEffect(() => {
    const element = root.current
    if (!element) return
    const measure = () => {
      if (expanded) return
      onCollapsedHeight?.(element.offsetHeight)
      const textElement = textRef.current, fieldsElement = fieldsRef.current
      setOverflows((!!textElement && textElement.scrollHeight > textElement.clientHeight + 1) || (!!fieldsElement && fieldsElement.scrollWidth > fieldsElement.clientWidth + 1))
    }
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [expanded, onCollapsedHeight, title, text, fields])
  useEffect(() => () => onCollapsedHeight?.(0), [onCollapsedHeight])

  const canExpand = overflows || !!children
  const toggleLabel = t(expanded ? 'messagehub.panel.collapse' : 'messagehub.panel.expand')
  return (
    <div className="pointer-events-none absolute inset-x-2 top-2 z-10 flex justify-center md:inset-x-4">
      <section ref={root} className="mh-session-panel pointer-events-auto" data-expanded={expanded || undefined} data-testid={testId} aria-label={title}>
        <div className="flex min-w-0 items-start gap-2">
          {icon ? <span className="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center text-[color:var(--cp-accent)]" aria-hidden>{icon}</span> : null}
          <div className="min-w-0 flex-1">
            <p className={`text-[13px] font-semibold leading-5 ${expanded ? 'break-words' : 'truncate'}`} data-testid="session-panel-title">{title}</p>
            {hasFields ? (
              <dl ref={fieldsRef} className={`flex gap-x-3 text-[12px] leading-5 ${expanded ? 'flex-wrap' : 'overflow-hidden whitespace-nowrap'}`} data-testid="session-panel-fields">
                {fields.map(field => <div key={field.label} className="flex shrink-0 gap-1"><dt className="text-[color:var(--cp-muted)]">{field.label}</dt><dd className="font-medium" style={field.tone ? { color: toneColor[field.tone] } : undefined}>{field.value}</dd></div>)}
              </dl>
            ) : null}
            {text ? <p ref={textRef} className="mh-session-panel-text text-[13px] leading-5" style={expanded ? undefined : { WebkitLineClamp: textLines }} data-clamped={!expanded || undefined} data-testid="session-panel-text">{text}</p> : null}
            {expanded && children ? <div className="mt-1.5 text-[13px] leading-5">{children}</div> : null}
          </div>
          {canExpand ? <button type="button" className="mh-session-panel-button" aria-expanded={expanded} aria-label={toggleLabel} title={toggleLabel} onClick={() => setExpanded(value => !value)} data-testid="session-panel-toggle">{expanded ? <ChevronUp size={16} /> : <ChevronDown size={16} />}</button> : null}
          {onDismiss ? <button type="button" className="mh-session-panel-button" aria-label={dismissLabel} title={dismissLabel} onClick={onDismiss} data-testid="session-panel-dismiss"><X size={16} /></button> : null}
        </div>
      </section>
    </div>
  )
}
