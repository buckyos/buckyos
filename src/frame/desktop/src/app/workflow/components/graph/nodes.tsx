/* ── Custom React Flow nodes for the workflow graph ── */

import { memo } from 'react'
import { Handle, Position, type NodeProps, type Node } from '@xyflow/react'
import { AlertTriangle, GitBranch, Layers, Repeat, User } from 'lucide-react'
import type {
  AnalysisIssue,
  ControlNodeView,
  TaskNodeView,
  WorkflowGraphNode,
} from '../../mock/types'

export interface WorkflowNodeData extends Record<string, unknown> {
  node: WorkflowGraphNode
  issues: AnalysisIssue[]
}

export type WorkflowFlowNode = Node<WorkflowNodeData, 'task' | 'control'>

const handleStyle = {
  opacity: 0,
  pointerEvents: 'none' as const,
  width: 6,
  height: 6,
}

function accent(node: WorkflowGraphNode) {
  if (node.kind === 'control') {
    return {
      fg: 'var(--cp-warning)',
      bg: 'color-mix(in srgb, var(--cp-warning) 14%, var(--cp-surface))',
      border: 'color-mix(in srgb, var(--cp-warning) 35%, var(--cp-border))',
    }
  }
  if (node.stepType !== 'autonomous') {
    return {
      fg: 'var(--cp-accent)',
      bg: 'color-mix(in srgb, var(--cp-accent) 14%, var(--cp-surface))',
      border: 'color-mix(in srgb, var(--cp-accent) 35%, var(--cp-border))',
    }
  }
  return {
    fg: 'var(--cp-text)',
    bg: 'var(--cp-surface)',
    border: 'var(--cp-border)',
  }
}

function controlIcon(node: ControlNodeView) {
  if (node.controlType === 'branch') return <GitBranch size={13} />
  if (node.controlType === 'parallel') return <Layers size={13} />
  return <Repeat size={13} />
}

function taskSubtitle(node: TaskNodeView): string {
  return node.executor?.raw ?? 'task'
}

function controlSubtitle(node: ControlNodeView): string {
  if (node.controlType === 'branch')
    return `branch · ${Object.keys(node.paths).length} paths`
  if (node.controlType === 'parallel')
    return `parallel · join=${node.join.strategy}${node.join.n ? `(${node.join.n})` : ''}`
  return `for_each · concurrency=${node.effectiveConcurrency}/${node.concurrency}`
}

interface BodyProps {
  node: WorkflowGraphNode
  issues: AnalysisIssue[]
  selected: boolean
  icon: React.ReactNode
  label: string
  subtitle: string
}

function NodeBody({ node, issues, selected, icon, label, subtitle }: BodyProps) {
  const color = accent(node)
  const hasErr = issues.some((i) => i.severity === 'error')
  const hasWarn = issues.some((i) => i.severity === 'warn')
  const showSeekableTag = node.kind === 'task' && node.outputMode !== 'single'

  return (
    <div
      style={{
        width: 200,
        height: 64,
        borderRadius: 10,
        background: color.bg,
        border: `${selected ? 2 : hasErr ? 1.6 : 1}px solid ${
          selected
            ? 'var(--cp-accent)'
            : hasErr
              ? 'var(--cp-danger)'
              : color.border
        }`,
        padding: '8px 10px',
        boxSizing: 'border-box',
        fontFamily: 'inherit',
        fontSize: 12,
        color: 'var(--cp-text)',
        lineHeight: 1.25,
        overflow: 'hidden',
        cursor: 'pointer',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 4,
          color: color.fg,
        }}
      >
        {icon}
        <span
          style={{
            fontSize: 10,
            textTransform: 'uppercase',
            letterSpacing: 1,
          }}
        >
          {label}
        </span>
        {showSeekableTag && node.kind === 'task' && (
          <span
            style={{
              marginLeft: 'auto',
              fontSize: 9,
              padding: '0 4px',
              borderRadius: 4,
              background: 'var(--cp-surface-2)',
              color: 'var(--cp-muted)',
              border: '1px solid var(--cp-border)',
            }}
          >
            {node.outputMode === 'finite_seekable' ? 'seekable' : 'sequential'}
          </span>
        )}
        {(hasErr || hasWarn) && (
          <span
            style={{
              marginLeft: showSeekableTag ? 4 : 'auto',
              color: hasErr ? 'var(--cp-danger)' : 'var(--cp-warning)',
              display: 'inline-flex',
              alignItems: 'center',
            }}
          >
            <AlertTriangle size={10} />
          </span>
        )}
      </div>
      <div
        style={{
          fontWeight: 600,
          marginTop: 2,
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}
      >
        {node.name}
      </div>
      <div
        style={{
          fontSize: 10,
          color: 'var(--cp-muted)',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}
      >
        {subtitle}
      </div>
    </div>
  )
}

function TaskNodeImpl({ data, selected }: NodeProps<WorkflowFlowNode>) {
  const node = data.node as TaskNodeView
  const icon = node.stepType !== 'autonomous' ? <User size={13} /> : null
  return (
    <>
      <Handle type="target" position={Position.Top} style={handleStyle} />
      <NodeBody
        node={node}
        issues={data.issues}
        selected={!!selected}
        icon={icon}
        label={node.stepType}
        subtitle={taskSubtitle(node)}
      />
      <Handle type="source" position={Position.Bottom} style={handleStyle} />
    </>
  )
}

function ControlNodeImpl({ data, selected }: NodeProps<WorkflowFlowNode>) {
  const node = data.node as ControlNodeView
  return (
    <>
      <Handle type="target" position={Position.Top} style={handleStyle} />
      <NodeBody
        node={node}
        issues={data.issues}
        selected={!!selected}
        icon={controlIcon(node)}
        label={node.controlType}
        subtitle={controlSubtitle(node)}
      />
      <Handle type="source" position={Position.Bottom} style={handleStyle} />
    </>
  )
}

export const TaskNode = memo(TaskNodeImpl)
export const ControlNode = memo(ControlNodeImpl)
