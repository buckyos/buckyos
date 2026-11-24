/* ── Workflow – app panel entry point ── */

import { useEffect, useState } from 'react'
import { WorkflowStoreContext } from './hooks/use-workflow-store'
import { WorkflowStore } from './api/store'
import { WorkflowShell } from './components/WorkflowShell'
import { workflowLaunchSchema } from './launch'
import type { AppContentLoaderProps } from '../types'

export function WorkflowAppPanel({ launch }: AppContentLoaderProps) {
  const parsed = workflowLaunchSchema.safeParse(launch?.payload)
  const [store] = useState(() => new WorkflowStore())
  useEffect(() => {
    void store.load()
    return () => store.dispose()
  }, [store])
  return (
    <WorkflowStoreContext.Provider value={store}>
      <WorkflowShell key={launch?.requestId ?? 'default'} initial={parsed.success ? parsed.data : undefined} />
    </WorkflowStoreContext.Provider>
  )
}
