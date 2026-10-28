/* Block-controls on the Desktop (脑图方案 §6.2, R2, R11): a container whose kind is a control id is laid out, drawn and
 * driven by its ContainerDefinition — the counterpart of the BlockRegistry for containers. The definition gives the
 * attributes (`configSchema`), the child rules, the named layout (from the shared module), the chrome the canvas draws
 * around the children (implicit edges, fold handles), how insertion creates an instance, its toolbar and keyboard
 * (the keys go to the control while one of its Blocks is selected) and the self-description Agents read. A kind with
 * no definition is drawn as an unrecognised group (R12). */

import type { ComponentType, ReactNode } from 'react'
import type { LucideIcon } from 'lucide-react'
import type { ControlDescriptor } from '@controls/catalog'
import type { ControlSource, SolveOptions, SolvedControl } from '@controls/solve'
import type { EntityEnvelope, Json, Operation } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import type { CanvasMode, CreateArgs, ToolbarItem } from '../blocks/registry'
import type { Laid } from '../canvas/layout'

/** What the chrome of a control is drawn with (in its container's frame, local coordinates). */
export interface ControlChromeProps {
  container: Laid
  laid: Map<string, Laid>
  mode: CanvasMode
  zoom: number
  /** A drag's proposed layout: node rectangles (world) to draw the edges of instead, and the node being dragged. */
  preview?: { rects: Map<string, { x: number; y: number; w: number; h: number }>; dragged: ReadonlySet<string>; parent: string | null; at: { x: number; y: number; w: number; h: number } | null } | null
}

/** What the control's keys and actions act with: the canvas's selection and editing. */
export interface ControlCommandContext {
  store: WorkspaceStore
  laid: Map<string, Laid>
  /** The selected Block inside the control (a node), or null when the container itself is selected. */
  nodeId: string | null
  containerId: string
  /** The Surface it is drawn on. */
  surface: EntityEnvelope
  select: (ids: string[]) => void
  /** Open the editor of a Block once it exists (a node just created). */
  edit: (id: string) => void
  canEdit: boolean
}

export interface ContainerDefinition {
  /** The control id: the container's `kind`. */
  kind: string
  version: number
  readonly title: string
  icon: LucideIcon
  /** `attr_def`: the container's `config` (checked by the core as well). */
  configSchema: Json
  children: {
    /** Views a node may have; `any` (R10). */
    accepts: string[] | 'any'
    /** Controls and groups as nodes (R24). */
    containers: boolean
    /** Named roles of the children (R22); none for a mind map. */
    slots?: { id: string; accepts: string[]; required?: boolean }[]
  }
  layout: {
    /** §5.7: one direction per container; v1 has bottom-up only. */
    direction: 'bottom_up' | 'top_down'
    algorithm: 'tree' | 'fixed'
    solve: (src: ControlSource, containerId: string, opts: SolveOptions) => SolvedControl
  }
  /** Drawn in the container's frame under its children: implicit edges, fold handles, pins (R23: template children). */
  Chrome?: ComponentType<ControlChromeProps>
  /** The whole control as one self-contained Block on a flow page (§6.3). */
  Flow?: ComponentType<{ containerId: string; mode: CanvasMode }>
  /** Insertion: the container and its first content children. `args.cellId` is the Block to select and edit. */
  create: (args: CreateArgs) => Operation[]
  /** The near toolbar of the selected container / of a selected node inside it. */
  toolbar?: (ctx: ControlCommandContext) => ToolbarItem[]
  /** Keys while the container or one of its Blocks is selected (true: handled). */
  keys?: (ctx: ControlCommandContext, event: KeyboardEvent) => boolean
  /** The properties panel of the container (direction, edges, spacing). */
  Inspector?: ComponentType<{ containerId: string; readOnly: string | null }>
  /** Help shown on hover of the container. */
  describe?: () => ReactNode
  /** What Agents read (R11, §8.1): the shared catalogue entry. */
  agent: ControlDescriptor
}

class ControlRegistry {
  private readonly defs = new Map<string, ContainerDefinition>()
  register(definition: ContainerDefinition) { this.defs.set(definition.kind, definition) }
  get(kind: string | null | undefined): ContainerDefinition | undefined { return kind ? this.defs.get(kind) : undefined }
  list(): ContainerDefinition[] { return [...this.defs.values()] }
}

export const controlRegistry = new ControlRegistry()

/** Container kinds of the two trees that are not drawn on a canvas. */
const NOT_BLOCK = new Set(['root', 'data', 'surfaces', 'folder', 'surface'])

/** A container drawn on a canvas: a group, a control, or a kind this client does not know (shown as a group, R12). */
export function isCanvasContainer(e: { type_id: string; kind?: string | null }): boolean {
  return e.type_id === 'buckyos.container' && !NOT_BLOCK.has(e.kind ?? '')
}
