/* ── Workflow graph canvas (React Flow + dagre layout) ── */

import { useEffect, useMemo } from 'react'
import {
  Background,
  BackgroundVariant,
  Controls,
  MarkerType,
  MiniMap,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type Edge,
  type Node,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'

import type {
  AnalysisIssue,
  WorkflowGraphView,
} from '../mock/types'
import { layoutGraph } from './graph/layout'
import {
  ControlNode,
  TaskNode,
  type WorkflowFlowNode,
  type WorkflowNodeData,
} from './graph/nodes'
import {
  WorkflowEdge,
  type WorkflowEdgeData,
  type WorkflowFlowEdge,
} from './graph/edges'

const nodeTypes = { task: TaskNode, control: ControlNode }
const edgeTypes = { workflow: WorkflowEdge }

interface GraphCanvasProps {
  graph: WorkflowGraphView
  issuesByNode: Record<string, AnalysisIssue[]>
  selectedNodeId: string | null
  onSelectNode: (nodeId: string | null) => void
}

const ARROW_NORMAL = {
  type: MarkerType.ArrowClosed,
  width: 14,
  height: 14,
  color: 'var(--cp-muted)',
} as const

const ARROW_IMPLICIT = {
  type: MarkerType.ArrowClosed,
  width: 14,
  height: 14,
  color: 'var(--cp-warning)',
} as const

function buildGraph(
  graph: WorkflowGraphView,
  issuesByNode: Record<string, AnalysisIssue[]>,
  selectedNodeId: string | null,
): { nodes: WorkflowFlowNode[]; edges: WorkflowFlowEdge[] } {
  const nodes: Node<WorkflowNodeData>[] = graph.nodes.map((n) => ({
    id: n.id,
    type: n.kind === 'control' ? 'control' : 'task',
    position: { x: 0, y: 0 },
    data: { node: n, issues: issuesByNode[n.id] ?? [] },
    selected: selectedNodeId === n.id,
    draggable: false,
    connectable: false,
  }))

  const edges: Edge<WorkflowEdgeData>[] = graph.edges
    .filter((e) => e.target)
    .map((e) => ({
      id: e.id,
      source: e.source,
      target: e.target as string,
      type: 'workflow',
      data: { implicit: e.implicit, conditionLabel: e.conditionLabel },
      markerEnd: e.implicit ? ARROW_IMPLICIT : ARROW_NORMAL,
    }))

  const laidOut = layoutGraph(nodes, edges) as WorkflowFlowNode[]
  return { nodes: laidOut, edges: edges as WorkflowFlowEdge[] }
}

function InnerGraphCanvas({
  graph,
  issuesByNode,
  selectedNodeId,
  onSelectNode,
}: GraphCanvasProps) {
  const { nodes, edges } = useMemo(
    () => buildGraph(graph, issuesByNode, selectedNodeId),
    [graph, issuesByNode, selectedNodeId],
  )

  const rf = useReactFlow()
  // Re-fit when the underlying definition changes (node/edge structure).
  useEffect(() => {
    const id = requestAnimationFrame(() => {
      rf.fitView({ padding: 0.15, duration: 200 })
    })
    return () => cancelAnimationFrame(id)
  }, [graph.definitionId, graph.definitionVersion, rf])

  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      nodeTypes={nodeTypes}
      edgeTypes={edgeTypes}
      fitView
      fitViewOptions={{ padding: 0.15 }}
      minZoom={0.3}
      maxZoom={1.5}
      nodesDraggable={false}
      nodesConnectable={false}
      elementsSelectable
      selectNodesOnDrag={false}
      panOnScroll
      proOptions={{ hideAttribution: true }}
      onNodeClick={(_, node) => onSelectNode(node.id)}
      onPaneClick={() => onSelectNode(null)}
      style={{ background: 'var(--cp-bg)' }}
    >
      <Background
        variant={BackgroundVariant.Lines}
        gap={24}
        color="color-mix(in srgb, var(--cp-border) 35%, transparent)"
      />
      <Controls showInteractive={false} />
      <MiniMap
        pannable
        zoomable
        maskColor="color-mix(in srgb, var(--cp-bg) 70%, transparent)"
        nodeColor={(n) => {
          const data = n.data as WorkflowNodeData | undefined
          if (!data) return 'var(--cp-surface)'
          if (data.node.kind === 'control')
            return 'color-mix(in srgb, var(--cp-warning) 40%, var(--cp-surface))'
          if (data.node.stepType !== 'autonomous')
            return 'color-mix(in srgb, var(--cp-accent) 40%, var(--cp-surface))'
          return 'var(--cp-surface)'
        }}
        style={{
          background: 'var(--cp-surface)',
          border: '1px solid var(--cp-border)',
        }}
      />
    </ReactFlow>
  )
}

export function GraphCanvas(props: GraphCanvasProps) {
  return (
    <div className="h-full w-full">
      <ReactFlowProvider>
        <InnerGraphCanvas {...props} />
      </ReactFlowProvider>
    </div>
  )
}
