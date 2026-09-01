import type {
  FinancialAggregate,
  FinancialEntry,
  FinancialReport,
} from "./types.ts";

type Usage = NonNullable<FinancialEntry["usage"]>;

function object(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : undefined;
}

function nonNegativeNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : undefined;
}

function nonNegativeInteger(value: unknown): number | undefined {
  const parsed = nonNegativeNumber(value);
  return parsed !== undefined && Number.isInteger(parsed) ? parsed : undefined;
}

function candidateResponses(value: unknown, depth = 0): Record<string, unknown>[] {
  if (depth > 6) return [];
  const current = object(value);
  if (!current) return [];
  const candidates = [current];
  for (const key of ["result", "output", "response", "summary"]) {
    candidates.push(...candidateResponses(current[key], depth + 1));
  }
  return candidates;
}

export function extractFinance(value: unknown): {
  usage?: Usage;
  actualCostUsd?: number;
  rawCostUsd?: number;
  creditAppliedUsd?: number;
} {
  let usage: Usage | undefined;
  let actualCostUsd: number | undefined;
  let rawCostUsd: number | undefined;
  let creditAppliedUsd: number | undefined;
  for (const candidate of candidateResponses(value)) {
    if (!usage) {
      const raw = object(candidate.usage);
      if (raw) {
        const parsed: Usage = {
          input_tokens: nonNegativeInteger(raw.input_tokens),
          output_tokens: nonNegativeInteger(raw.output_tokens),
          total_tokens: nonNegativeInteger(raw.total_tokens),
          request_units: nonNegativeInteger(raw.request_units),
        };
        if (Object.values(parsed).some((item) => item !== undefined)) usage = parsed;
      }
    }
    if (actualCostUsd === undefined) {
      const cost = object(candidate.cost);
      const currency = typeof cost?.currency === "string" ? cost.currency.toUpperCase() : "";
      if (currency === "USD") actualCostUsd = nonNegativeNumber(cost?.amount);
    }
    const billing = object(object(candidate.extra)?.billing);
    rawCostUsd ??= nonNegativeNumber(billing?.raw_cost_usd);
    creditAppliedUsd ??= nonNegativeNumber(billing?.sn_ai_provider_credit_applied_usd);
  }
  return { usage, actualCostUsd, rawCostUsd, creditAppliedUsd };
}

export type CostReservation = { id: number; estimatedCostUsd: number };

export class CostBudget {
  private nextId = 1;
  private readonly reservations = new Map<number, number>();
  private settledExposureUsd = 0;
  private exceeded = false;
  private readonly budgetUsd: number;

  constructor(budgetUsd: number) {
    if (!Number.isFinite(budgetUsd) || budgetUsd < 0) throw new Error("budget must be non-negative");
    this.budgetUsd = budgetUsd;
  }

  reserve(estimatedCostUsd: number): CostReservation {
    if (!Number.isFinite(estimatedCostUsd) || estimatedCostUsd < 0) {
      throw new Error("estimated cost must be non-negative");
    }
    if (this.exposureUsd() + estimatedCostUsd > this.budgetUsd + Number.EPSILON) {
      throw new Error(
        `financial budget exhausted: exposure $${this.exposureUsd().toFixed(6)} + reservation $${estimatedCostUsd.toFixed(6)} > $${this.budgetUsd.toFixed(6)}`,
      );
    }
    const reservation = { id: this.nextId++, estimatedCostUsd };
    this.reservations.set(reservation.id, estimatedCostUsd);
    return reservation;
  }

  settle(reservation: CostReservation, actualCostUsd?: number): void {
    if (!this.reservations.delete(reservation.id)) throw new Error("unknown financial reservation");
    const exposure = actualCostUsd ?? reservation.estimatedCostUsd;
    if (!Number.isFinite(exposure) || exposure < 0) throw new Error("actual cost must be non-negative");
    this.settledExposureUsd += exposure;
    if (this.exposureUsd() > this.budgetUsd + Number.EPSILON) this.exceeded = true;
  }

  exposureUsd(): number {
    return this.settledExposureUsd + [...this.reservations.values()].reduce((sum, item) => sum + item, 0);
  }

  budgetExceeded(): boolean {
    return this.exceeded;
  }
}

function aggregate(entries: readonly FinancialEntry[], keyOf: (entry: FinancialEntry) => string): FinancialAggregate[] {
  const values = new Map<string, FinancialAggregate>();
  for (const entry of entries) {
    const key = keyOf(entry);
    const value = values.get(key) ?? {
      key,
      calls: 0,
      input_tokens: 0,
      output_tokens: 0,
      total_tokens: 0,
      request_units: 0,
      actual_cost_usd: 0,
      raw_cost_usd: 0,
      credit_applied_usd: 0,
      estimated_exposure_usd: 0,
      unknown_cost_calls: 0,
    };
    value.calls += 1;
    value.input_tokens += entry.usage?.input_tokens ?? 0;
    value.output_tokens += entry.usage?.output_tokens ?? 0;
    value.total_tokens += entry.usage?.total_tokens ?? 0;
    value.request_units += entry.usage?.request_units ?? 0;
    if (entry.actual_cost_usd !== undefined) value.actual_cost_usd += entry.actual_cost_usd;
    else {
      value.estimated_exposure_usd += entry.estimated_cost_usd;
      value.unknown_cost_calls += 1;
    }
    value.raw_cost_usd += entry.raw_cost_usd ?? entry.actual_cost_usd ?? 0;
    value.credit_applied_usd += entry.credit_applied_usd ?? 0;
    values.set(key, value);
  }
  return [...values.values()].sort((left, right) => left.key.localeCompare(right.key));
}

export function buildFinancialReport(input: {
  entries: readonly FinancialEntry[];
  budgetUsd: number;
  plannedMaxCalls: number;
  plannedMaxCostUsd: number;
  budgetExceeded?: boolean;
}): FinancialReport {
  const entries = [...input.entries].sort((left, right) =>
    left.case_id.localeCompare(right.case_id) || left.attempt - right.attempt
  );
  const actualCost = entries.reduce((sum, item) => sum + (item.actual_cost_usd ?? 0), 0);
  const rawCost = entries.reduce(
    (sum, item) => sum + (item.raw_cost_usd ?? item.actual_cost_usd ?? 0),
    0,
  );
  const creditApplied = entries.reduce((sum, item) => sum + (item.credit_applied_usd ?? 0), 0);
  const estimatedExposure = entries.reduce(
    (sum, item) => sum + (item.actual_cost_usd === undefined ? item.estimated_cost_usd : 0),
    0,
  );
  const totalExposure = actualCost + estimatedExposure;
  return {
    currency: "USD",
    budget_usd: input.budgetUsd,
    planned_max_calls: input.plannedMaxCalls,
    planned_max_cost_usd: input.plannedMaxCostUsd,
    actual_calls: entries.length,
    actual_cost_usd: actualCost,
    raw_cost_usd: rawCost,
    credit_applied_usd: creditApplied,
    estimated_exposure_usd: estimatedExposure,
    total_exposure_usd: totalExposure,
    remaining_budget_usd: Math.max(0, input.budgetUsd - totalExposure),
    unknown_cost_calls: entries.filter((item) => item.actual_cost_usd === undefined).length,
    budget_exceeded: Boolean(input.budgetExceeded) || totalExposure > input.budgetUsd + Number.EPSILON,
    entries,
    by_provider: aggregate(entries, (entry) => entry.provider_driver),
    by_instance: aggregate(entries, (entry) => `${entry.provider_driver}/${entry.provider_instance}`),
    by_model: aggregate(entries, (entry) => entry.exact_model),
    by_api_type: aggregate(entries, (entry) => entry.api_type),
    by_case: aggregate(entries, (entry) => entry.case_id),
  };
}
