import type { ModelCatalog } from '../datamodel/model-catalog'
import {
  commandSubject,
  commandValue,
  isExactTarget,
  targetBase,
  type AiccEvent,
  type AiccEventFeed,
  type DirectoryKind,
  type ExpansionStep,
  type FilteredCandidate,
  type RankedCandidate,
  type RoutePreview,
  type RoutingCommand,
  type RoutingCommandStatus,
  type RoutingDirectory,
  type RoutingItem,
  type RoutingState,
  type RoutingWorkspace,
} from '../datamodel/routing'
import type { ModelMetadata, StoreSnapshot } from './types'

interface MockNode {
  path: string
  kind: DirectoryKind
  apiType: string
  profile?: string
  items: RoutingItem[]
}

const TASKS: { path: string; profile: string; items: [string, string, number][] }[] = [
  { path: 'llm.chat', profile: 'balanced', items: [['gpt-standard', 'llm.gpt-standard', 2], ['sonnet', 'llm.sonnet', 2], ['gpt-mini', 'llm.gpt-mini', 1.5], ['qwen-dense-27b', 'llm.qwen-dense-27b', 1]] },
  { path: 'llm.code', profile: 'local_first', items: [['sonnet', 'llm.sonnet', 2.4], ['gpt-standard', 'llm.gpt-standard', 2.1], ['qwen-coder', 'llm.qwen-coder', 1.8]] },
  { path: 'llm.plan', profile: 'quality_first', items: [['opus', 'llm.opus', 2.5], ['gpt-standard', 'llm.gpt-standard', 2.2], ['qwen-coder', 'llm.qwen-coder', 1.1]] },
]

const COST_SCORE: Record<string, number> = { low: 0, medium: 0.5, high: 1 }

export class MockRoutingWorld {
  private commands: RoutingCommand[] = []
  private revision = 1
  private events: AiccEvent[] = []
  private nextEventId = 1
  private stale = new Map<string, AiccEvent>()

  workspace(snapshot: StoreSnapshot, catalog: ModelCatalog): RoutingWorkspace {
    const { nodes, status } = this.build(snapshot, catalog)
    const models = modelIndex(snapshot)
    const directories: RoutingDirectory[] = [...nodes.values()].map((node) => {
      const preview = simulate(nodes, models, node.path)
      return {
        path: node.path,
        kind: node.kind,
        apiType: node.apiType,
        profile: node.profile,
        items: node.items,
        available: preview.available,
        selectedExactModel: preview.selectedExactModel,
        error: preview.error,
      }
    })
    return { ...this.state(status), directories }
  }

  routingState(snapshot: StoreSnapshot, catalog: ModelCatalog): RoutingState {
    return this.state(this.build(snapshot, catalog).status)
  }

  preview(snapshot: StoreSnapshot, catalog: ModelCatalog, path: string): RoutePreview {
    const { nodes } = this.build(snapshot, catalog)
    return simulate(nodes, modelIndex(snapshot), path)
  }

  save(snapshot: StoreSnapshot, catalog: ModelCatalog, commands: RoutingCommand[], expectedRevision: number): RoutingState {
    if (expectedRevision !== this.revision) throw new Error('settings_revision_conflict')
    this.commands = commands
    this.revision += 1
    this.pushEvent('info', 'routing_commands_updated', `routing adjustments updated (${commands.length} active)`, { count: commands.length })
    return this.routingState(snapshot, catalog)
  }

  eventFeed(limit: number): AiccEventFeed {
    return { events: [...this.events].reverse().slice(0, limit), activeWarnings: [...this.stale.values()] }
  }

  private state(status: RoutingCommandStatus[]): RoutingState {
    return { settingsRevision: this.revision, commands: this.commands, status }
  }

  private pushEvent(level: AiccEvent['level'], kind: string, message: string, details: Record<string, unknown>): AiccEvent {
    const command = details.command as RoutingCommand | undefined
    const event = { id: this.nextEventId++, createdAtMs: Date.now(), level, kind, message, details, command }
    this.events = [...this.events.slice(-199), event]
    return event
  }

  private build(snapshot: StoreSnapshot, catalog: ModelCatalog) {
    const nodes = baseTree(snapshot, catalog)
    const status = applyCommands(nodes, catalog, this.commands)
    const current = new Map(status.filter((entry) => entry.staleReason).map((entry) => [commandSubject(entry.command), entry]))
    for (const [key, event] of this.stale) {
      if (current.has(key)) continue
      this.stale.delete(key)
      this.pushEvent('info', 'routing_command_recovered', `routing adjustment applies again: ${key}`, { command: event.details.command })
    }
    for (const [key, entry] of current) {
      if (this.stale.has(key)) continue
      this.stale.set(key, this.pushEvent('warning', 'routing_command_stale', `routing adjustment no longer applies: ${entry.staleReason}`, { command: entry.command, reason: entry.staleReason }))
    }
    return { nodes, status }
  }
}

function baseTree(snapshot: StoreSnapshot, catalog: ModelCatalog): Map<string, MockNode> {
  const nodes = new Map<string, MockNode>()
  const item = (name: string, target: string, weight: number, source: string): RoutingItem => ({
    name, target, weight, defaultWeight: weight, source, weightSource: source,
  })
  for (const task of TASKS) {
    nodes.set(task.path, {
      path: task.path, kind: 'task', apiType: 'llm', profile: task.profile,
      items: task.items.map(([name, target, weight]) => item(name, target, weight, 'builtin_definition')),
    })
  }
  for (const vendor of catalog.vendors) {
    for (const spec of vendor.specs) {
      const members = spec.members.filter((member) => vendor.models.find((model) => model.id === member.model_id)?.providers.length)
      nodes.set(spec.path, {
        path: spec.path, kind: 'spec', apiType: 'llm',
        items: members.map((member) => item(member.target.split(':')[0], member.target, versionWeight(member.model_id), 'driver_metadata_mount')),
      })
      for (const member of members) {
        const model = vendor.models.find((entry) => entry.id === member.model_id)!
        const family = targetBase(member.target)
        nodes.set(family, {
          path: family, kind: 'family', apiType: 'llm',
          items: model.providers.flatMap((provider) => provider.exact_models).map((exact) => item(exact, exact, 1, 'driver_metadata_mount')),
        })
      }
    }
  }
  const embeddings = allModels(snapshot).filter((model) => model.api_types.includes('embedding.text'))
  if (embeddings.length) {
    nodes.set('embedding.text', {
      path: 'embedding.text', kind: 'task', apiType: 'embedding.text', profile: 'latency_first',
      items: embeddings.map((model) => item(model.exact_model, model.exact_model, 1, 'auto_admission')),
    })
  }
  return nodes
}

function applyCommands(nodes: Map<string, MockNode>, catalog: ModelCatalog, commands: RoutingCommand[]): RoutingCommandStatus[] {
  const vendorOf = new Map(catalog.vendors.flatMap((vendor) => vendor.models.map((model) => [model.id, vendor.id] as const)))
  const specVendor = new Map(catalog.vendors.flatMap((vendor) => vendor.specs.map((spec) => [spec.path, vendor.id] as const)))
  const leaves = (target: string, trail = new Set<string>()): string[] => {
    if (isExactTarget(target)) {
      const model = target.split('@')[0]
      return [`${vendorOf.get(model) ?? ''}/${model}`]
    }
    const node = nodes.get(targetBase(target))
    if (!node || trail.has(node.path)) return []
    return node.items.flatMap((entry) => leaves(entry.target, new Set([...trail, node.path])))
  }
  const ordered = [...commands.filter((command) => command.kind !== 'item_weight'), ...commands.filter((command) => command.kind === 'item_weight')]
  return ordered.map((command) => {
    const staleReason = staleReasonOf(command, nodes, catalog)
    let matchedItems = 0
    if (!staleReason) {
      for (const node of nodes.values()) {
        node.items = node.items.map((entry) => {
          const base = targetBase(entry.target)
          const owned = leaves(entry.target)
          const matched = command.kind === 'item_weight'
            ? command.path === node.path && command.item === entry.name
            : command.kind === 'spec_factor'
              ? command.spec === base
              : command.kind === 'vendor_factor'
                ? specVendor.get(base) === command.vendor || (owned.length > 0 && owned.every((leaf) => leaf.startsWith(`${command.vendor}/`)))
                : owned.length > 0 && owned.every((leaf) => leaf === `${command.vendor}/${command.model}`)
          if (!matched) return entry
          matchedItems += 1
          return {
            ...entry,
            weight: command.kind === 'item_weight' ? command.weight : entry.weight * commandValue(command),
            weightSource: 'routing_command',
          }
        })
      }
    }
    return { command, matchedItems, staleReason }
  })
}

function staleReasonOf(command: RoutingCommand, nodes: Map<string, MockNode>, catalog: ModelCatalog): string | undefined {
  switch (command.kind) {
    case 'vendor_factor':
      return catalog.vendors.some((vendor) => vendor.id === command.vendor) ? undefined : `vendor ${command.vendor} is not in the model catalog`
    case 'spec_factor':
      return catalog.vendors.some((vendor) => vendor.specs.some((spec) => spec.path === command.spec)) ? undefined : `specification ${command.spec} no longer exists`
    case 'model_factor':
      return catalog.vendors.some((vendor) => vendor.id === command.vendor && vendor.models.some((model) => model.id === command.model))
        ? undefined
        : `model ${command.vendor}/${command.model} is not in the model catalog`
    case 'item_weight': {
      const node = nodes.get(command.path)
      if (!node) return `directory ${command.path} no longer exists`
      return node.items.some((entry) => entry.name === command.item) ? undefined : `item ${command.item} no longer exists in ${command.path}`
    }
  }
}

function simulate(nodes: Map<string, MockNode>, models: Map<string, ModelMetadata>, path: string): RoutePreview {
  const steps: ExpansionStep[] = []
  const filtered: FilteredCandidate[] = []
  const expand = (target: string, trail: Set<string>): string[] => {
    const node = nodes.get(targetBase(target))
    if (!node || trail.has(node.path)) return []
    const items = node.items.filter((entry) => entry.weight > 0)
    const weights = [...new Set(items.map((entry) => entry.weight))].sort((left, right) => right - left)
    let pool: string[] = []
    let selected: number | undefined
    for (const weight of weights) {
      for (const entry of items.filter((candidate) => candidate.weight === weight)) {
        if (isExactTarget(entry.target)) {
          const model = models.get(entry.target)
          if (model && model.health.status !== 'unavailable') pool = [...new Set([...pool, entry.target])]
          else if (!filtered.some((candidate) => candidate.exactModel === entry.target)) {
            filtered.push({ exactModel: entry.target, reasons: [{ code: 'provider_unavailable', summary: 'provider is unavailable' }] })
          }
        } else {
          pool = [...new Set([...pool, ...expand(entry.target, new Set([...trail, node.path]))])]
        }
      }
      if (pool.length) {
        selected = weight
        break
      }
    }
    if (!steps.some((step) => step.path === target)) {
      steps.push({
        path: target,
        maxWeight: selected,
        items: items.map((entry) => ({
          name: entry.name,
          target: entry.target,
          weight: entry.weight,
          weightSource: entry.weightSource,
          state: selected == null ? 'unavailable' : entry.weight === selected ? 'expanded' : entry.weight < selected ? 'not_expanded' : 'unavailable',
        })),
      })
    }
    return pool
  }
  const node = nodes.get(path)
  const pool = expand(path, new Set())
  const ranked: RankedCandidate[] = pool.map((exact, index) => {
    const model = models.get(exact)
    const cost = COST_SCORE[model?.attributes.cost_class ?? ''] ?? 1
    const local = node?.profile === 'local_first' && !model?.attributes.local ? 1 : 0
    return {
      exactModel: exact,
      providerInstanceName: exact.split('@')[1] ?? '',
      defaultOrder: index,
      finalScore: node?.profile === 'local_first' ? cost * 0.3 + local * 0.7 : cost,
      selected: false,
      scoreInputs: { cost, quality: 0, preference: 0, cache: 0, local },
    }
  }).sort((left, right) => left.finalScore - right.finalScore || left.defaultOrder - right.defaultOrder)
  if (ranked[0]) ranked[0] = { ...ranked[0], selected: true }
  return {
    path,
    available: ranked.length > 0,
    selectedExactModel: ranked[0]?.exactModel,
    error: ranked.length ? undefined : `no candidate for ${path}`,
    schedulerProfile: node?.profile ?? 'balanced',
    expansion: steps,
    ranked,
    filtered,
    fallbackChain: [],
  }
}

function allModels(snapshot: StoreSnapshot): ModelMetadata[] {
  return [
    ...snapshot.providers.filter((provider) => provider.config.enabled).flatMap((provider) => provider.status.discovered_models),
    ...snapshot.localModels,
  ]
}

function modelIndex(snapshot: StoreSnapshot): Map<string, ModelMetadata> {
  return new Map(allModels(snapshot).map((model) => [model.exact_model, model]))
}

function versionWeight(modelId: string): number {
  const match = modelId.match(/(\d+)(?:\.(\d+))?/)
  return match ? Number(match[1]) * 10 + Number(match[2] ?? 0) : 1
}
