/* Mount budget (phase two §9.3 rule 8): how many editors and HTML runtimes may be mounted at once. */

import { createContext, useContext, useEffect, useState } from 'react'

export interface MountBudget { editors: number; html: number }
export const DEFAULT_BUDGET: MountBudget = { editors: 6, html: 4 }

export class BudgetCounter {
  readonly limits: MountBudget
  private readonly counts = { editors: 0, html: 0 }
  private readonly listeners = new Set<() => void>()
  private version = 0
  constructor(limits: MountBudget) { this.limits = limits }
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener) } }
  snapshot = () => this.version
  take(kind: keyof MountBudget): boolean {
    if (this.counts[kind] >= this.limits[kind]) return false
    this.counts[kind] += 1
    this.bump()
    return true
  }
  give(kind: keyof MountBudget) { this.counts[kind] = Math.max(0, this.counts[kind] - 1); this.bump() }
  used(kind: keyof MountBudget) { return this.counts[kind] }
  private bump() { this.version += 1; for (const listener of [...this.listeners]) listener() }
}

export const BudgetContext = createContext<BudgetCounter>(new BudgetCounter(DEFAULT_BUDGET))
export function createBudget(limits: MountBudget = DEFAULT_BUDGET) { return new BudgetCounter(limits) }

/** Reserve one slot of the budget while mounted; `false` when the limit is reached (no kind asked: `true`). */
export function useBudgetSlot(kind: keyof MountBudget | null): boolean {
  const budget = useContext(BudgetContext)
  const [granted, setGranted] = useState<boolean>(false)
  useEffect(() => {
    if (!kind) return
    let held = false
    const attempt = () => { if (!held && budget.take(kind)) { held = true; setGranted(true) } }
    attempt()
    const off = budget.subscribe(attempt)
    return () => { off(); if (held) budget.give(kind); setGranted(false) }
  }, [budget, kind])
  return kind ? granted : true
}
