/* ── Workflow – app panel entry point ── */

import { useState } from 'react'
import type { AppContentLoaderProps } from '../types'
import { WorkflowStoreContext } from './hooks/use-workflow-store'
import { WorkflowMockStore } from './mock/store'
import { WorkflowShell } from './components/WorkflowShell'

export function WorkflowAppPanel(_props: AppContentLoaderProps) {
  const [store] = useState(() => new WorkflowMockStore())
  return (
    <WorkflowStoreContext.Provider value={store}>
      <WorkflowShell />
    </WorkflowStoreContext.Provider>
  )
}
