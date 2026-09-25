import { candidateTree, upsertCommand, winReason, type RoutePreview } from '../../src/app/ai-center/datamodel/routing.ts'

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}

function preview(scores: [string, number][]): RoutePreview {
  return {
    path: 'llm.chat',
    available: true,
    selectedExactModel: scores[0][0],
    schedulerProfile: 'cost_first',
    expansion: [
      { path: 'llm.chat', maxWeight: 2, items: [
        { name: 'pro', target: 'llm.gpt-pro', weight: 2, weightSource: 'builtin_definition', state: 'expanded' },
        { name: 'mini', target: 'llm.gpt-mini', weight: 1, weightSource: 'builtin_definition', state: 'not_expanded' },
      ] },
      { path: 'llm.gpt-pro', maxWeight: 56, items: scores.map(([exact]) => ({ name: exact, target: exact, weight: 56, weightSource: 'driver_metadata_mount', state: 'expanded' as const })) },
    ],
    ranked: scores.map(([exactModel, finalScore], index) => ({
      exactModel,
      providerInstanceName: exactModel.split('@')[1],
      defaultOrder: index,
      finalScore,
      selected: index === 0,
      scoreInputs: { cost: finalScore, quality: 0, preference: 0, cache: 0, local: 0 },
    })),
    filtered: [],
    fallbackChain: [],
  }
}

Deno.test('win reason distinguishes single, scored and tied winners', () => {
  assert(winReason(preview([['a@x', 0]])).code === 'only_candidate', 'single candidate')
  const scored = winReason(preview([['a@x', 0.1], ['b@y', 0.6]]))
  assert(scored.code === 'best_score' && scored.factor === 'cost' && scored.runnerUp === 'b@y', 'cost decides')
  assert(winReason(preview([['a@x', 0.5], ['b@y', 0.5]])).code === 'default_order', 'tie keeps default order')
})

Deno.test('candidate tree keeps the path to the winner and skipped siblings', () => {
  const { nodes } = candidateTree(preview([['a@x', 0.1], ['b@y', 0.6]]))
  assert(nodes.length === 2 && nodes[1].item.state === 'not_expanded', 'root level lists every item')
  assert(nodes[0].winning && nodes[0].children[0].winning && !nodes[0].children[1].winning, 'winning path is marked')
  assert(nodes[0].children[0].ranked?.selected === true, 'leaf carries ranking')
})

Deno.test('commands are replaced by subject and removable', () => {
  const first = upsertCommand([], { kind: 'vendor_factor', vendor: 'claude', factor: 2 })
  const second = upsertCommand(first, { kind: 'vendor_factor', vendor: 'claude', factor: 3 })
  assert(second.length === 1 && second[0].kind === 'vendor_factor' && second[0].factor === 3, 'replaced')
  assert(upsertCommand(second, second[0], true).length === 0, 'removed')
})
