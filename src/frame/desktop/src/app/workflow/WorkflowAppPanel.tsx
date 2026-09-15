/* ── Workflow – app panel entry point ── */

import { useState } from 'react'
import { WorkflowStoreContext } from './hooks/use-workflow-store'
import { WorkflowMockStore } from './mock/store'
import { WorkflowShell } from './components/WorkflowShell'

export function WorkflowAppPanel() {
  const [store] = useState(() => new WorkflowMockStore())
  return (
    <WorkflowStoreContext.Provider value={store}>
      <WorkflowShell />
    </WorkflowStoreContext.Provider>
  )
}
