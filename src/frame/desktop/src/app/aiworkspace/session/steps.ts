/* The steps a session plays, resolved on the run Workspace like the third phase's stage. */

import { useMemo } from 'react'
import type { ShowPathPayload } from '../api/types'
import { playable, resolveSteps, type ResolvedStep } from '../presentation/model'
import { useOutlineVersion } from '../state/hooks'
import type { WorkspaceStore } from '../state/store'
import type { SessionStage } from './types'

export function useSteps(store: WorkspaceStore, path: ShowPathPayload) {
  const outlineVersion = useOutlineVersion()
  // eslint-disable-next-line react-hooks/exhaustive-deps -- outlineVersion is the invalidation signal
  return useMemo(() => playable(resolveSteps(store.outline, path)), [store, path, outlineVersion])
}

/** Where the confirmed stage is among the steps this terminal resolved: by step id, else by position. */
export function stepOf(steps: ResolvedStep[], stage: SessionStage): number {
  const byId = steps.findIndex((s) => s.step.id === stage.step_id)
  if (byId >= 0) return byId
  return Math.max(0, Math.min(stage.index ?? 0, steps.length - 1))
}
