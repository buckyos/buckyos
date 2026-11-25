import { Button } from '@mui/material'
import type { ReactNode } from 'react'
import { useI18n } from '../../../../i18n/provider'
import { SectionNote } from '../shared/Section'
import { TONE_COLORS, type DomainFailure, type Tone } from './labels'

export function Dot({ tone }: { tone: Tone }) {
  return <span aria-hidden className="mt-2 h-2 w-2 shrink-0 rounded-full" style={{ background: TONE_COLORS[tone] }} />
}

/** A status dot, a title with its state at the right, and what is below them. */
export function StatusLine({ tone, title, state, children, testId }: { tone: Tone; title: ReactNode; state?: string; children?: ReactNode; testId?: string }) {
  return (
    <div className="flex gap-2.5 py-2" data-testid={testId} data-tone={tone}>
      <Dot tone={tone} />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-baseline justify-between gap-x-3">
          <span className="min-w-0 break-words text-sm font-medium leading-6" style={{ color: 'var(--cp-text)' }}>{title}</span>
          {state ? <span className="text-xs leading-6" style={{ color: tone === 'pass' || tone === 'neutral' ? 'var(--cp-muted)' : TONE_COLORS[tone] }}>{state}</span> : null}
        </div>
        {children}
      </div>
    </div>
  )
}

export function Muted({ children }: { children: ReactNode }) {
  return <p className="text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>{children}</p>
}

/** Technical text (a hostname, a record value) that wraps anywhere and stays left to right. */
export function Mono({ children }: { children: ReactNode }) {
  return <code dir="ltr" className="break-all font-mono text-xs leading-5" style={{ color: 'var(--cp-text)' }}>{children}</code>
}

/** A raw message, folded away. */
export function Details({ children }: { children: string }) {
  const { t } = useI18n()
  return (
    <details className="text-xs" style={{ color: 'var(--cp-muted)' }}>
      <summary className="cursor-pointer">{t('settings.domain.details')}</summary>
      <p dir="ltr" className="mt-1 break-words">{children}</p>
    </details>
  )
}

export function Failure({ failure, onRetry, testId }: { failure: DomainFailure | null; onRetry?: () => void; testId?: string }) {
  const { t } = useI18n()
  if (!failure) return null
  return (
    <div className="space-y-1" data-testid={testId}>
      <SectionNote tone="danger">{failure.text}</SectionNote>
      {failure.detail ? <Details>{failure.detail}</Details> : null}
      {onRetry ? <Button size="small" variant="outlined" onClick={onRetry}>{t('settings.zone.retry')}</Button> : null}
    </div>
  )
}

export function StepTitle({ children }: { children: ReactNode }) {
  return <h4 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{children}</h4>
}
