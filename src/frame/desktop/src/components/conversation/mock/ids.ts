/** The viewer of the mock runtime. */
export const MOCK_SELF_DID = 'did:buckyos:user:self'

/** Task ids the seeded Agent replies of `session-coder-task` carry in `agent_task` (served by `mock/tasks.ts`). */
export const MOCK_AGENT_TASKS = {
  done: 'task-turn-report',
  fast: 'task-turn-clock',
  failed: 'task-turn-deploy',
  foreign: 'task-turn-foreign',
  running: 'task-turn-checklist',
  /** Waits for the answer to a question (`session-coder-ask`). */
  asking: 'task-turn-docs',
  /** Asked two questions, then replied (`session-coder-qa`). */
  answered: 'task-turn-notes',
  /** Only ever carried by an edit, so it must never be read. */
  hijack: 'task-turn-hijack',
  /** UI Turns of `session-coder-work` that created work sessions (`mock/worksessions.ts`). */
  workSimple: 'task-turn-work-builds',
  workPlan: 'task-turn-work-docs',
  workDone: 'task-turn-work-notes',
  /** Still analysing the request: no sub-task, no work session yet. */
  workAnalyze: 'task-turn-work-deploy',
} as const
