/* ── Shared run / definition display helpers ── */

import type { RunStatus, WorkflowDefinition } from '../types'

export const runStatusStyle: Record<RunStatus, { label: string; color: string }> = {
  created: { label: 'Created', color: 'var(--cp-muted)' },
  running: { label: 'Running', color: 'var(--cp-accent)' },
  waiting_human: { label: 'Waiting Human', color: 'var(--cp-warning)' },
  completed: { label: 'Completed', color: 'var(--cp-success)' },
  failed: { label: 'Failed', color: 'var(--cp-danger)' },
  paused: { label: 'Paused', color: 'var(--cp-warning)' },
  aborted: { label: 'Aborted', color: 'var(--cp-muted)' },
  budget_exhausted: { label: 'Budget Exhausted', color: 'var(--cp-danger)' },
}

export function formatTime(iso: string) {
  return new Date(iso).toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

export const sourceLabels: Record<WorkflowDefinition['source'], string> = {
  system: 'system',
  user_imported: 'imported',
  app_registered: 'app',
  agent_generated: 'agent',
}

export function statusBadge(status: WorkflowDefinition['status']): {
  label: string
  bg: string
  fg: string
} {
  switch (status) {
    case 'draft':
      return {
        label: 'draft',
        bg: 'color-mix(in srgb, var(--cp-warning) 18%, transparent)',
        fg: 'var(--cp-warning)',
      }
    case 'active':
      return {
        label: 'active',
        bg: 'color-mix(in srgb, var(--cp-success) 18%, transparent)',
        fg: 'var(--cp-success)',
      }
    case 'archived':
      return {
        label: 'archived',
        bg: 'color-mix(in srgb, var(--cp-muted) 22%, transparent)',
        fg: 'var(--cp-muted)',
      }
  }
}
