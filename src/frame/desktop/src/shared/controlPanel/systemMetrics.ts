import type { ZoneChannel } from '../channel.ts'
import { callControlPanel } from './call.ts'

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

export function readSystemMetrics(channel: ZoneChannel): Promise<SystemMetrics> {
  return callControlPanel<SystemMetrics>(channel, 'system.metrics', { lite: true })
}
