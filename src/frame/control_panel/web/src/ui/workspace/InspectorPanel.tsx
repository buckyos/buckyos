import Icon from '../icons'
import { useWorkspace } from './WorkspaceContext'
import StepDetail from './inspector/StepDetail'
import TaskDetail from './inspector/TaskDetail'
import WorkLogDetail from './inspector/WorkLogDetail'
import TodoDetail from './inspector/TodoDetail'
import SubAgentDetail from './inspector/SubAgentDetail'

const kindLabels: Record<string, string> = {
  step: 'Step Detail',
  task: 'Task Detail',
  worklog: 'WorkLog Detail',
  todo: 'Todo Detail',
  'sub-agent': 'Sub-Agent Detail',
}

const InspectorPanel = () => {
  const { inspectorTarget, closeInspector } = useWorkspace()

  if (!inspectorTarget) return null

  const renderContent = () => {
    switch (inspectorTarget.kind) {
      case 'step':
        return <StepDetail step={inspectorTarget.data} />
      case 'task':
        return <TaskDetail task={inspectorTarget.data} />
      case 'worklog':
        return <WorkLogDetail log={inspectorTarget.data} />
      case 'todo':
        return <TodoDetail todo={inspectorTarget.data} />
      case 'sub-agent':
        return <SubAgentDetail agent={inspectorTarget.data} />
      default:
        return null
    }
  }

  return (
    <aside className="flex h-full w-[400px] flex-none flex-col border-l border-[var(--cp-border)] bg-white">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-[var(--cp-border)] px-4 py-3">
        <div className="flex items-center gap-2">
          <h3 className="text-sm font-semibold text-[var(--cp-ink)]">
            {kindLabels[inspectorTarget.kind] ?? 'Detail'}
          </h3>
          <span className="cp-pill border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]">
            {inspectorTarget.kind}
          </span>
        </div>
        <button
          type="button"
          onClick={closeInspector}
          className="rounded-lg p-1 text-[var(--cp-muted)] transition hover:bg-[var(--cp-surface-muted)] hover:text-[var(--cp-ink)]"
        >
          <Icon name="close" className="size-4" />
        </button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-4">{renderContent()}</div>
    </aside>
  )
}

export default InspectorPanel
