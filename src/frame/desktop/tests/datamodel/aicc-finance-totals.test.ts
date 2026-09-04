import { normalizeFinanceTotals } from '../../src/app/ai-center/datamodel/transforms.ts'

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}

Deno.test('finance totals preserve currencies, merge normalized duplicates, and rank the largest amount first', () => {
  const totals = normalizeFinanceTotals([
    { amount: 4.5, currency: 'eur' },
    { amount: 2, currency: 'USD' },
    { amount: 3, currency: ' usd ' },
    { amount: Number.NaN, currency: 'CNY' },
  ])
  assert(totals.length === 2, 'invalid currency totals must be discarded')
  assert(totals[0].currency === 'USD' && totals[0].amount === 5, 'the largest normalized total must be first')
  assert(totals[1].currency === 'EUR' && totals[1].amount === 4.5, 'currencies must remain separate')
})

Deno.test('finance total normalization scales to one million dashboard rows', () => {
  for (const count of [1, 10, 1_000, 1_000_000]) {
    const input = Array.from({ length: count }, (_, index) => ({
      amount: 0.01,
      currency: index % 2 === 0 ? 'USD' : 'EUR',
    }))
    const startedAt = performance.now()
    const totals = normalizeFinanceTotals(input)
    const elapsed = performance.now() - startedAt
    assert(totals.length <= 2, `${count} rows must aggregate to at most two currencies`)
    console.log(`[AICC finance totals] ${count} rows: ${elapsed.toFixed(1)}ms`)
  }
})
