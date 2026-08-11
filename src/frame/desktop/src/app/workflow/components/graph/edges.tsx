/* ── Custom React Flow edge with optional condition label ── */

import { memo } from 'react'
import {
  BaseEdge,
  EdgeLabelRenderer,
  getBezierPath,
  type Edge,
  type EdgeProps,
} from '@xyflow/react'

export interface WorkflowEdgeData extends Record<string, unknown> {
  implicit?: boolean
  conditionLabel?: string
}

export type WorkflowFlowEdge = Edge<WorkflowEdgeData, 'workflow'>

function WorkflowEdgeImpl({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  data,
  markerEnd,
}: EdgeProps<WorkflowFlowEdge>) {
  const [path, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition,
  })

  const implicit = !!data?.implicit
  const stroke = implicit ? 'var(--cp-warning)' : 'var(--cp-muted)'

  return (
    <>
      <BaseEdge
        id={id}
        path={path}
        markerEnd={markerEnd}
        style={{
          stroke,
          strokeWidth: 1.4,
          strokeDasharray: implicit ? '5,4' : undefined,
        }}
      />
      {data?.conditionLabel && (
        <EdgeLabelRenderer>
          <div
            style={{
              position: 'absolute',
              transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)`,
              padding: '1px 6px',
              borderRadius: 6,
              background: 'var(--cp-surface)',
              border: '1px solid var(--cp-border)',
              color: 'var(--cp-warning)',
              fontFamily: 'ui-monospace, monospace',
              fontSize: 10,
              lineHeight: '14px',
              pointerEvents: 'none',
              whiteSpace: 'nowrap',
            }}
          >
            {data.conditionLabel}
          </div>
        </EdgeLabelRenderer>
      )}
    </>
  )
}

export const WorkflowEdge = memo(WorkflowEdgeImpl)
