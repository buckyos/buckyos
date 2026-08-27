import type { RpcClient } from "./gateway.ts";

export type UsageEvent = {
  event_id: string;
  task_id: string;
  caller_app_id?: string;
  capability: string;
  request_model: string;
  provider_model: string;
  input_tokens?: number;
  output_tokens?: number;
  total_tokens?: number;
  request_units?: number;
  usage_json: Record<string, unknown>;
  finance_snapshot_json?: Record<string, unknown>;
  created_at_ms: number;
};

export type RouteTraceEvent = {
  trace_id: string;
  task_id: string;
  selected_exact_model?: string;
  provider_instance_name?: string;
  api_type: string;
  created_at_ms: number;
};

export function providerInstanceFromExactModel(exactModel: string): string | undefined {
  const separator = exactModel.lastIndexOf("@");
  return separator >= 0 && separator < exactModel.length - 1
    ? exactModel.slice(separator + 1)
    : undefined;
}

export function providerCoverage(input: {
  exactModels: readonly string[];
  inventories: readonly { provider_instance_name: string; provider_driver: string }[];
  expectedDrivers: readonly string[];
}): {
  observedInstances: string[];
  observedDrivers: string[];
  missingExpectedDrivers: string[];
} {
  const driverByInstance = new Map(input.inventories.map((inventory) => [
    inventory.provider_instance_name,
    inventory.provider_driver,
  ]));
  const observedInstances = [...new Set(input.exactModels.flatMap((exactModel) => {
    const instance = providerInstanceFromExactModel(exactModel);
    return instance ? [instance] : [];
  }))].sort();
  const observedDrivers = [...new Set(observedInstances.flatMap((instance) => {
    const driver = driverByInstance.get(instance);
    return driver ? [driver] : [];
  }))].sort();
  return {
    observedInstances,
    observedDrivers,
    missingExpectedDrivers: input.expectedDrivers.filter((driver) =>
      !observedDrivers.includes(driver)
    ),
  };
}

function eventsFrom(value: unknown): { events: UsageEvent[]; nextCursor?: string } {
  if (!value || typeof value !== "object") throw new Error("usage.query returned non-object");
  const response = value as { events?: unknown; next_cursor?: unknown };
  if (response.events !== undefined && !Array.isArray(response.events)) {
    throw new Error("usage.query.events must be an array");
  }
  const rawEvents = Array.isArray(response.events) ? response.events : [];
  const events = rawEvents.map((item) => {
    if (!item || typeof item !== "object") throw new Error("usage.query contains invalid event");
    const event = item as UsageEvent;
    if (!event.event_id || !event.task_id || !event.provider_model) {
      throw new Error("usage event is missing identity fields");
    }
    return event;
  });
  return {
    events,
    nextCursor: typeof response.next_cursor === "string" && response.next_cursor
      ? response.next_cursor
      : undefined,
  };
}

export async function queryUsageEvents(input: {
  aicc: RpcClient;
  startTimeMs: number;
  endTimeMs: number;
  taskIds?: string[];
  callerAppIds?: string[];
  limit?: number;
}): Promise<UsageEvent[]> {
  const all: UsageEvent[] = [];
  let cursor: string | undefined;
  const seen = new Set<string>();
  do {
    const raw = await input.aicc.call("usage.query", {
      time_range: {
        kind: "explicit",
        start_time_ms: input.startTimeMs,
        end_time_ms: input.endTimeMs,
      },
      filters: {
        task_ids: input.taskIds ?? [],
        caller_app_ids: input.callerAppIds ?? [],
      },
      output_mode: "events",
      limit: input.limit ?? 500,
      ...(cursor ? { cursor } : {}),
    });
    const page = eventsFrom(raw);
    for (const event of page.events) {
      if (seen.has(event.event_id)) throw new Error(`usage.query repeated event ${event.event_id}`);
      seen.add(event.event_id);
      all.push(event);
    }
    cursor = page.nextCursor;
  } while (cursor);
  return all;
}

function number(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : undefined;
}

export function usageEventFinance(event: UsageEvent): {
  usage: {
    input_tokens?: number;
    output_tokens?: number;
    total_tokens?: number;
    request_units?: number;
  };
  actualCostUsd?: number;
  rawCostUsd?: number;
  creditAppliedUsd?: number;
} {
  const snapshot = event.finance_snapshot_json ?? {};
  const billing = snapshot.billing && typeof snapshot.billing === "object"
    ? snapshot.billing as Record<string, unknown>
    : {};
  const currency = typeof snapshot.currency === "string" ? snapshot.currency.toUpperCase() : "";
  return {
    usage: {
      input_tokens: number(event.input_tokens),
      output_tokens: number(event.output_tokens),
      total_tokens: number(event.total_tokens),
      request_units: number(event.request_units),
    },
    actualCostUsd: currency === "USD" ? number(snapshot.amount) : undefined,
    rawCostUsd: number(billing.raw_cost_usd),
    creditAppliedUsd: number(billing.sn_ai_provider_credit_applied_usd),
  };
}

export function indexUsageByTask(events: readonly UsageEvent[]): Map<string, UsageEvent[]> {
  const indexed = new Map<string, UsageEvent[]>();
  for (const event of events) {
    const values = indexed.get(event.task_id) ?? [];
    values.push(event);
    indexed.set(event.task_id, values);
  }
  return indexed;
}

export async function queryRouteTraces(input: {
  aicc: RpcClient;
  startTimeMs: number;
  endTimeMs: number;
  taskIds: string[];
  limit?: number;
}): Promise<RouteTraceEvent[]> {
  const all: RouteTraceEvent[] = [];
  const seen = new Set<string>();
  let cursor: string | undefined;
  do {
    const raw = await input.aicc.call("trace.query", {
      start_time_ms: input.startTimeMs,
      end_time_ms: input.endTimeMs,
      task_ids: input.taskIds,
      limit: input.limit ?? 500,
      ...(cursor ? { cursor } : {}),
    });
    if (!raw || typeof raw !== "object") throw new Error("trace.query returned non-object");
    const response = raw as { traces?: unknown; next_cursor?: unknown };
    if (response.traces !== undefined && !Array.isArray(response.traces)) {
      throw new Error("trace.query.traces must be an array");
    }
    const rawTraces = Array.isArray(response.traces) ? response.traces : [];
    for (const rawTrace of rawTraces) {
      if (!rawTrace || typeof rawTrace !== "object") throw new Error("trace.query contains invalid trace");
      const trace = rawTrace as RouteTraceEvent;
      if (!trace.trace_id || !trace.task_id) throw new Error("route trace is missing identity fields");
      if (seen.has(trace.trace_id)) throw new Error(`trace.query repeated trace ${trace.trace_id}`);
      seen.add(trace.trace_id);
      all.push(trace);
    }
    cursor = typeof response.next_cursor === "string" && response.next_cursor
      ? response.next_cursor
      : undefined;
  } while (cursor);
  return all;
}
