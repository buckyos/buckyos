/* ── Workflow – app panel entry point ── */

import { useEffect, useState } from 'react'
import { WorkflowStoreContext } from './hooks/use-workflow-store'
import { WorkflowStore } from './api/store'
import { WorkflowShell } from './components/WorkflowShell'

export function WorkflowAppPanel() {
  const [store] = useState(() => new WorkflowStore())
  useEffect(() => {
    void store.load()
    return () => store.dispose()
  }, [store])
  return (
    <WorkflowStoreContext.Provider value={store}>
      <WorkflowShell />
    </WorkflowStoreContext.Provider>
  )
}
