/* ── TaskCenter: task rows and the expandable task tree ── */

import { useRef, useState, type KeyboardEvent } from 'react'
import { ChevronRight, MessageSquare } from 'lucide-react'
import { notifyCardPath, type Task } from '../../../api/task_mgr'
import { Meter, StatusDot } from './taskView'
import { liveProgress, taskTone, type TaskText } from './taskModel'

interface TaskRowProps {
  task: Task
  text: TaskText
  selected?: boolean
  depth?: number
  /** Tree rows only: true / false for a node with subtasks, null for a leaf. */
  expanded?: boolean | null
  onToggle?: () => void
  onSelect: () => void
  role?: 'treeitem' | 'button'
  tabIndex?: number
  onKeyDown?: (event: KeyboardEvent<HTMLDivElement>) => void
  onFocus?: () => void
  rowRef?: (element: HTMLDivElement | null) => void
}

export function TaskRow({
  task,
  text,
  selected = false,
  depth = 0,
  expanded,
  onToggle,
  onSelect,
  role = 'button',
  tabIndex = 0,
  onKeyDown,
  onFocus,
  rowRef,
}: TaskRowProps) {
  const progress = liveProgress(task)
  const tone = taskTone(task)
  const tree = role === 'treeitem'

  return (
    <div
      ref={rowRef}
      role={role}
      tabIndex={tabIndex}
      className="tc-row"
      data-depth={depth}
      data-child={depth > 0 ? '' : undefined}
      data-task-id={task.taskId}
      style={{ '--tc-depth': depth } as React.CSSProperties}
      aria-level={tree ? depth + 1 : undefined}
      aria-expanded={tree && expanded != null ? expanded : undefined}
      aria-selected={tree ? selected : undefined}
      aria-current={!tree && selected ? 'true' : undefined}
      onClick={onSelect}
      onFocus={onFocus}
      onKeyDown={(event) => {
        if (onKeyDown) onKeyDown(event)
        else if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault()
          onSelect()
        }
      }}
    >
      {expanded !== undefined && (
        <button
          type="button"
          tabIndex={-1}
          className="tc-toggle"
          data-spacer={expanded === null ? '' : undefined}
          aria-expanded={expanded ?? undefined}
          aria-label={text.t(expanded ? 'taskCenter.tree.collapse' : 'taskCenter.tree.expand')}
          onClick={(event) => {
            event.stopPropagation()
            onToggle?.()
          }}
        >
          <ChevronRight size={14} strokeWidth={1.75} />
        </button>
      )}
      <span className="tc-row-dot">
        <StatusDot tone={tone} />
      </span>
      <span className="tc-row-body">
        <span className="tc-row-title">{task.title}</span>
        <span className="tc-row-meta">{text.meta(task)}</span>
      </span>
      <span className="tc-row-aside">
        <span>{text.listTime(task.updatedAt)}</span>
        {task.notifyId ? (
          <a
            href={notifyCardPath(task.notifyId)}
            className="tc-link"
            data-testid="task-notify-link"
            onClick={(event) => event.stopPropagation()}
          >
            <MessageSquare size={12} strokeWidth={1.75} />
            {text.t('taskCenter.notify.open', 'Handle in MessageHub')}
          </a>
        ) : progress != null ? (
          <span className="tc-row-progress">
            <Meter value={progress} tone={tone} />
            <span className="tc-num">{progress}%</span>
          </span>
        ) : null}
      </span>
    </div>
  )
}

interface VisibleRow {
  task: Task
  depth: number
  parentId: string | null
}

interface TaskTreeProps {
  roots: Task[]
  text: TaskText
  isOpen: (taskId: string) => boolean
  onToggle: (taskId: string) => void
  selectedId?: string
  onSelect: (taskId: string) => void
  /** Arrow keys open the focused task too: on a desktop with the detail panel showing. */
  followFocus?: boolean
}

export function TaskTree({ roots, text, isOpen, onToggle, selectedId, onSelect, followFocus = false }: TaskTreeProps) {
  const [focusId, setFocusId] = useState<string | null>(null)
  const elements = useRef(new Map<string, HTMLDivElement>())

  const rows: VisibleRow[] = []
  const walk = (tasks: Task[], depth: number, parentId: string | null) => {
    for (const task of tasks) {
      rows.push({ task, depth, parentId })
      if (task.children.length > 0 && isOpen(task.taskId)) walk(task.children, depth + 1, task.taskId)
    }
  }
  walk(roots, 0, null)

  const has = (id: string | null | undefined) => id != null && rows.some((row) => row.task.taskId === id)
  const activeId = has(focusId) ? focusId : has(selectedId) ? selectedId : rows[0]?.task.taskId

  const focus = (id: string, select: boolean) => {
    setFocusId(id)
    elements.current.get(id)?.focus()
    if (select) onSelect(id)
  }

  const handleKey = (event: KeyboardEvent<HTMLDivElement>, index: number) => {
    const row = rows[index]
    const id = row.task.taskId
    const parent = row.task.children.length > 0
    const move = (target: VisibleRow | undefined) => {
      if (!target) return
      event.preventDefault()
      focus(target.task.taskId, followFocus)
    }
    switch (event.key) {
      case 'ArrowDown':
        move(rows[index + 1])
        break
      case 'ArrowUp':
        move(rows[index - 1])
        break
      case 'Home':
        move(rows[0])
        break
      case 'End':
        move(rows[rows.length - 1])
        break
      case 'ArrowRight':
        event.preventDefault()
        if (parent && !isOpen(id)) onToggle(id)
        else if (parent) move(rows[index + 1])
        break
      case 'ArrowLeft':
        event.preventDefault()
        if (parent && isOpen(id)) onToggle(id)
        else if (row.parentId) move(rows.find((item) => item.task.taskId === row.parentId))
        break
      case 'Enter':
      case ' ':
        event.preventDefault()
        onSelect(id)
        break
      default:
        break
    }
  }

  return (
    <div role="tree" className="tc-list" aria-label={text.t('taskCenter.nav.tasks', 'Tasks')} data-testid="task-tree">
      {rows.map((row, index) => {
        const id = row.task.taskId
        return (
          <TaskRow
            key={id}
            task={row.task}
            text={text}
            depth={row.depth}
            role="treeitem"
            tabIndex={id === activeId ? 0 : -1}
            selected={id === selectedId}
            expanded={row.task.children.length > 0 ? isOpen(id) : null}
            onToggle={() => onToggle(id)}
            onSelect={() => onSelect(id)}
            onFocus={() => setFocusId(id)}
            onKeyDown={(event) => handleKey(event, index)}
            rowRef={(element) => {
              if (element) elements.current.set(id, element)
              else elements.current.delete(id)
            }}
          />
        )
      })}
    </div>
  )
}
