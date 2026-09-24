export interface CurrencyAmount {
  amount: number
  currency: string
}

export function normalizeFinanceTotals(value: unknown): CurrencyAmount[] {
  if (!Array.isArray(value)) return []
  const totals = new Map<string, number>()
  for (const item of value) {
    if (item == null || typeof item !== 'object' || Array.isArray(item)) continue
    const record = item as Record<string, unknown>
    const currency = typeof record.currency === 'string' ? record.currency.trim().toUpperCase() : ''
    const amount = typeof record.amount === 'number' && Number.isFinite(record.amount) ? record.amount : null
    if (!currency || amount == null) continue
    totals.set(currency, (totals.get(currency) ?? 0) + amount)
  }
  return Array.from(totals, ([currency, amount]) => ({ currency, amount }))
    .sort((left, right) => right.amount - left.amount || left.currency.localeCompare(right.currency))
}
