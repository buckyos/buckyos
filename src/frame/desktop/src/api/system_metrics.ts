import { isMockRuntime } from '../runtime.ts'
import { callRpc } from './rpc.ts'

/** The `system.metrics` fields the shell shows (control_panel `handle_system_metrics`, `lite` form). */
export interface SystemMetrics {
  cpu: { usagePercent: number; model: string; cores: number }
  memory: { totalGb: number; usedGb: number; usagePercent: number }
  disk: { totalGb: number; usedGb: number; usagePercent: number }
  network: { rxPerSec: number; txPerSec: number }
  loadAverage: { one: number; five: number; fifteen: number }
  processCount: number
  uptimeSeconds: number
}

const mockBootedAt = Date.now() - 3 * 86_400_000 - 5 * 3_600_000

/** A plausible, slowly drifting node for the mock runtime. */
function mockSystemMetrics(): SystemMetrics {
  const phase = Date.now() / 60_000
  const cpu = Math.round(18 + 9 * Math.sin(phase * 2.1) + 4 * Math.sin(phase * 7.3))
  return {
    cpu: { usagePercent: cpu, model: 'AMD Ryzen 7 5825U', cores: 16 },
    memory: { totalGb: 32, usedGb: 13.4, usagePercent: 42 },
    disk: { totalGb: 1863, usedGb: 721, usagePercent: 39 },
    network: {
      rxPerSec: Math.round(420_000 + 260_000 * Math.sin(phase * 3.7)),
      txPerSec: Math.round(96_000 + 40_000 * Math.sin(phase * 5.1)),
    },
    loadAverage: { one: 1.12, five: 0.94, fifteen: 0.81 },
    processCount: 286,
    uptimeSeconds: Math.round((Date.now() - mockBootedAt) / 1000),
  }
}

export async function fetchSystemMetrics(): Promise<{ data: SystemMetrics | null; error: unknown }> {
  if (isMockRuntime()) return { data: mockSystemMetrics(), error: null }
  return callRpc<SystemMetrics>('system.metrics', { lite: true })
}
