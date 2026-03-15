import Icon from '../icons'
import { WorkspaceProvider, useWorkspace } from './WorkspaceContext'
import WorkspaceSidebar from './WorkspaceSidebar'
import WorkspaceHeader from './WorkspaceHeader'
import InspectorPanel from './InspectorPanel'
import OverviewTab from './tabs/OverviewTab'
import WorkLogTab from './tabs/WorkLogTab'
import TasksTab from './tabs/TasksTab'
import TodosTab from './tabs/TodosTab'
import SubAgentsTab from './tabs/SubAgentsTab'

const tabs: { id: WsTabId; label: string; icon: IconName }[] = [
  { id: 'overview', label: 'Overview', icon: 'dashboard' },
  { id: 'worklog', label: 'WorkLog', icon: 'chart' },
  { id: 'tasks', label: 'Tasks', icon: 'spark' },
  { id: 'todos', label: 'Todos', icon: 'todo' },
  { id: 'sub-agents', label: 'Sub-Agents', icon: 'branch' },
]

const TabContent = () => {
  const { activeTab } = useWorkspace()
  switch (activeTab) {
    case 'overview':
      return <OverviewTab />
    case 'worklog':
      return <WorkLogTab />
    case 'tasks':
      return <TasksTab />
    case 'todos':
      return <TodosTab />
    case 'sub-agents':
      return <SubAgentsTab />
    default:
      return null
  }
}

const WorkspaceInner = () => {
  const { activeTab, setActiveTab, inspectorTarget } = useWorkspace()

  return (
    <div className="flex h-screen overflow-hidden bg-[var(--cp-bg)]">
      {/* Left sidebar */}
      <div className="w-[260px] flex-none overflow-y-auto">
        <WorkspaceSidebar />
      </div>

      {/* Main content */}
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        {/* Header */}
        <div className="flex-none px-6 pt-4">
          <WorkspaceHeader />
        </div>

        {/* Tab bar */}
        <div className="flex-none border-b border-[var(--cp-border)] px-6">
          <div className="flex gap-0.5 pt-3">
            {tabs.map((tab) => (
              <button
                key={tab.id}
                type="button"
                onClick={() => setActiveTab(tab.id)}
                className={`flex items-center gap-1.5 rounded-t-xl px-4 py-2.5 text-xs font-semibold transition ${
                  activeTab === tab.id
                    ? 'border border-b-0 border-[var(--cp-border)] bg-white text-[var(--cp-ink)] shadow-sm'
                    : 'text-[var(--cp-muted)] hover:bg-[var(--cp-surface-muted)] hover:text-[var(--cp-ink)]'
                }`}
              >
                <Icon name={tab.icon} className="size-3.5" />
                {tab.label}
              </button>
            ))}
          </div>
        </div>

        {/* Tab content */}
        <div className="flex-1 overflow-y-auto px-6 py-5">
          <TabContent />
        </div>
      </div>

      {/* Right inspector */}
      {inspectorTarget && (
        <div className="flex-none overflow-y-auto">
          <InspectorPanel />
        </div>
      )}
    </div>
  )
}

const WorkspaceLayout = () => {
  return (
    <WorkspaceProvider>
      <WorkspaceInner />
    </WorkspaceProvider>
  )
}

export default WorkspaceLayout
