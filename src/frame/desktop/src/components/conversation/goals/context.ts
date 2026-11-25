import { createContext } from 'react'
import type { DID } from 'buckyos'
import type { WorklogDestination } from '../worklog/model'
import type { WorkCard } from '../worksessions/model'
import type { Goal } from './model'

/** What the rows of one Goal's conversation share. */
export interface GoalRowsState {
  agentDid: DID
  viewerDid: DID
  goal: Goal
  now: number
  works: Record<string, WorkCard>
  /** Set while what is said to the Goal waits for a budget: when it resumes, if that is known. */
  hold: { until_ms?: number } | null
  /** The unfolded processes, by Activation; kept here because a row out of view is not mounted. */
  unfolded: ReadonlySet<string>
  toggle: (id: string) => void
  onOpenWorklog: (destination: WorklogDestination) => void
}

export const GoalRowsContext = createContext<GoalRowsState | null>(null)
