import { Boxes, Compass, Folder, Layers } from 'lucide-react'

export type DirectoryKind = 'task' | 'spec' | 'family' | 'directory'

export const KIND_STYLE: Record<DirectoryKind, { color: string; icon: typeof Compass }> = {
  task: { color: '#8b5cf6', icon: Compass },
  spec: { color: '#0ea5e9', icon: Layers },
  family: { color: '#f59e0b', icon: Boxes },
  directory: { color: '#64748b', icon: Folder },
}

export function kindLabel(kind: DirectoryKind, t: (key: string, fallback?: string) => string): string {
  return t(`aiCenter.routing.kind.${kind}`, kind)
}

export interface RoutingItem {
  name: string
  target: string
  weight: number
  defaultWeight: number
  source: string
  weightSource: string
}

export interface RoutingDirectory {
  path: string
  kind: DirectoryKind
  apiType?: string
  profile?: string
  items: RoutingItem[]
  available: boolean
  selectedExactModel?: string
  error?: string
}

export type RoutingCommand =
  | { kind: 'vendor_factor'; vendor: string; factor: number }
  | { kind: 'spec_factor'; spec: string; factor: number }
  | { kind: 'model_factor'; vendor: string; model: string; factor: number }
  | { kind: 'item_weight'; path: string; item: string; weight: number }

export interface RoutingCommandStatus {
  command: RoutingCommand
  matchedItems: number
  staleReason?: string
}

export interface RoutingState {
  settingsRevision: number
  commands: RoutingCommand[]
  status: RoutingCommandStatus[]
}

export interface RoutingWorkspace extends RoutingState {
  directories: RoutingDirectory[]
}

export type ExpansionState = 'expanded' | 'unavailable' | 'not_expanded'

export interface ExpansionItem {
  name: string
  target: string
  weight: number
  weightSource: string
  state: ExpansionState
}

export interface ExpansionStep {
  path: string
  maxWeight?: number
  items: ExpansionItem[]
}

export interface ScoreInputs {
  cost: number
  quality: number
  preference: number
  cache: number
  local: number
}

export interface RankedCandidate {
  exactModel: string
  providerInstanceName: string
  defaultOrder: number
  finalScore: number
  selected: boolean
  scoreInputs?: ScoreInputs
}

export interface FilteredCandidate {
  exactModel: string
  reasons: { code: string; summary: string }[]
}

export interface RoutePreview {
  path: string
  available: boolean
  selectedExactModel?: string
  error?: string
  schedulerProfile?: string
  expansion: ExpansionStep[]
  ranked: RankedCandidate[]
  filtered: FilteredCandidate[]
  fallbackChain: { from: string; to: string; reason: string }[]
}

export type AiccEventLevel = 'info' | 'warning' | 'error'

export interface AiccEvent {
  id: number
  createdAtMs: number
  level: AiccEventLevel
  kind: string
  message: string
  details: Record<string, unknown>
  command?: RoutingCommand
}

export interface AiccEventFeed {
  events: AiccEvent[]
  activeWarnings: AiccEvent[]
}

export type WinReason =
  | { code: 'only_candidate' }
  | { code: 'best_score'; runnerUp: string; factor: keyof ScoreInputs | 'score' }
  | { code: 'default_order'; runnerUp: string }
  | { code: 'none' }

export const SCORE_KEYS: (keyof ScoreInputs)[] = ['cost', 'quality', 'preference', 'cache', 'local']

export function commandSubject(command: RoutingCommand): string {
  switch (command.kind) {
    case 'vendor_factor': return `vendor:${command.vendor}`
    case 'spec_factor': return `spec:${command.spec}`
    case 'model_factor': return `model:${command.vendor}/${command.model}`
    case 'item_weight': return `item:${command.path}/${command.item}`
  }
}

export function commandValue(command: RoutingCommand): number {
  return command.kind === 'item_weight' ? command.weight : command.factor
}

export function withCommandValue<T extends RoutingCommand>(command: T, value: number): T {
  return command.kind === 'item_weight' ? { ...command, weight: value } : { ...command, factor: value }
}

export function findCommand<T extends RoutingCommand>(commands: RoutingCommand[], subject: T): T | undefined {
  const key = commandSubject(subject)
  return commands.find((command) => commandSubject(command) === key) as T | undefined
}

export function upsertCommand(commands: RoutingCommand[], next: RoutingCommand, remove = false): RoutingCommand[] {
  const key = commandSubject(next)
  const rest = commands.filter((command) => commandSubject(command) !== key)
  return remove ? rest : [...rest, next]
}

export function commandStatus(state: RoutingState | undefined, command: RoutingCommand): RoutingCommandStatus | undefined {
  if (!state) return undefined
  const key = commandSubject(command)
  return state.status.find((status) => commandSubject(status.command) === key)
}

export function isExactTarget(target: string): boolean {
  return target.includes('@')
}

export function targetBase(target: string): string {
  return isExactTarget(target) ? target : target.split(':')[0]
}

export function providerOfExact(exactModel: string): string {
  return exactModel.split('@')[1] ?? ''
}

export function modelOfExact(exactModel: string): string {
  return exactModel.split('@')[0] ?? exactModel
}

export function lastSegment(path: string): string {
  const base = targetBase(path)
  return base.slice(base.lastIndexOf('.') + 1) || base
}

export function namespaceOf(path: string): string {
  return path.split('.')[0] ?? path
}

export function winReason(preview: RoutePreview): WinReason {
  const ranked = [...preview.ranked].sort((left, right) => left.finalScore - right.finalScore || left.defaultOrder - right.defaultOrder)
  const winner = ranked.find((candidate) => candidate.selected) ?? ranked[0]
  if (!winner) return { code: 'none' }
  const others = ranked.filter((candidate) => candidate !== winner)
  if (others.length === 0) return { code: 'only_candidate' }
  const runnerUp = others[0]
  if (Math.abs(runnerUp.finalScore - winner.finalScore) < 1e-9) {
    return { code: 'default_order', runnerUp: runnerUp.exactModel }
  }
  let factor: keyof ScoreInputs | 'score' = 'score'
  if (winner.scoreInputs && runnerUp.scoreInputs) {
    let best = 0
    for (const key of SCORE_KEYS) {
      const gain = runnerUp.scoreInputs[key] - winner.scoreInputs[key]
      if (gain > best + 1e-9) {
        best = gain
        factor = key
      }
    }
  }
  return { code: 'best_score', runnerUp: runnerUp.exactModel, factor }
}

export interface CandidateTreeNode {
  key: string
  item: ExpansionItem
  step?: ExpansionStep
  children: CandidateTreeNode[]
  ranked?: RankedCandidate
  filtered?: FilteredCandidate
  winning: boolean
}

export function candidateTree(preview: RoutePreview): { root?: ExpansionStep; nodes: CandidateTreeNode[] } {
  const steps = new Map(preview.expansion.map((step) => [step.path, step]))
  const ranked = new Map(preview.ranked.map((candidate) => [candidate.exactModel, candidate]))
  const filtered = new Map(preview.filtered.map((candidate) => [candidate.exactModel, candidate]))
  const selected = preview.selectedExactModel
  const build = (step: ExpansionStep, trail: Set<string>): CandidateTreeNode[] => step.items.map((item) => {
    const key = `${step.path}/${item.name}`
    const childStep = !isExactTarget(item.target) && !trail.has(item.target) ? steps.get(item.target) : undefined
    const children = childStep && item.state === 'expanded'
      ? build(childStep, new Set([...trail, item.target]))
      : []
    const leaf = isExactTarget(item.target)
    return {
      key,
      item,
      step: childStep,
      children,
      ranked: leaf ? ranked.get(item.target) : undefined,
      filtered: leaf ? filtered.get(item.target) : undefined,
      winning: leaf ? item.target === selected : children.some((child) => child.winning),
    }
  })
  const rootPath = preview.fallbackChain.at(-1)?.to ?? preview.path
  const root = steps.get(rootPath) ?? steps.get(preview.path)
  return { root, nodes: root ? build(root, new Set([root.path])) : [] }
}
