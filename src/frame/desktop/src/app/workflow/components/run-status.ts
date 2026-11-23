/* ── Shared run / definition display helpers ── */

import type { RunStatus, WorkflowDefinition } from '../types'

export const runStatusStyle: Record<RunStatus, { color: string }> = {
  created: { color: 'var(--cp-muted)' },
  running: { color: 'var(--cp-accent)' },
  waiting_human: { color: 'var(--cp-warning)' },
  completed: { color: 'var(--cp-success)' },
  failed: { color: 'var(--cp-danger)' },
  paused: { color: 'var(--cp-warning)' },
  aborted: { color: 'var(--cp-muted)' },
  budget_exhausted: { color: 'var(--cp-danger)' },
}

export function formatTime(iso: string, locale: string) {
  return new Date(iso).toLocaleString(locale, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

export function statusBadge(status: WorkflowDefinition['status']): {
  bg: string
  fg: string
} {
  switch (status) {
    case 'draft':
      return {
        bg: 'color-mix(in srgb, var(--cp-warning) 18%, transparent)',
        fg: 'var(--cp-warning)',
      }
    case 'active':
      return {
        bg: 'color-mix(in srgb, var(--cp-success) 18%, transparent)',
        fg: 'var(--cp-success)',
      }
    case 'archived':
      return {
        bg: 'color-mix(in srgb, var(--cp-muted) 22%, transparent)',
        fg: 'var(--cp-muted)',
      }
  }
}
