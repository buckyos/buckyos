import { Check } from 'lucide-react'
import { useMediaQuery } from '@mui/material'

interface StepperProps {
  steps: string[]
  current: number
}

export function Stepper({ steps, current }: StepperProps) {
  const isMobile = useMediaQuery('(max-width: 767px)')

  if (isMobile) {
    return (
      <div className="flex flex-col gap-1 py-2">
        <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
          {`${current + 1} / ${steps.length} · ${steps[current]}`}
        </span>
        <div className="h-1 rounded-full w-full" style={{ background: 'var(--cp-border)' }}>
          <div
            className="h-1 rounded-full transition-all"
            style={{
              width: `${((current + 1) / steps.length) * 100}%`,
              background: 'var(--cp-accent)',
            }}
          />
        </div>
      </div>
    )
  }

  return (
    <div className="flex items-center gap-1">
      {steps.map((label, i) => {
        const done = i < current
        const active = i === current
        return (
          <div key={label} className="flex items-center gap-1">
            {i > 0 && (
              <div
                className="w-8 h-px mx-1"
                style={{ background: done ? 'var(--cp-accent)' : 'var(--cp-border)' }}
              />
            )}
            <div className="flex items-center gap-2">
              <div
                className="w-7 h-7 rounded-full flex items-center justify-center text-xs font-medium shrink-0"
                style={{
                  background: done
                    ? 'var(--cp-success)'
                    : active
                      ? 'var(--cp-accent)'
                      : 'var(--cp-border)',
                  color: done || active ? '#fff' : 'var(--cp-muted)',
                }}
              >
                {done ? <Check size={14} /> : i + 1}
              </div>
              <span
                className="text-xs whitespace-nowrap"
                style={{ color: active ? 'var(--cp-text)' : 'var(--cp-muted)' }}
              >
                {label}
              </span>
            </div>
          </div>
        )
      })}
    </div>
  )
}
