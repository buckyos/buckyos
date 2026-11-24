import type { ZoneChannel } from '../channel.ts'

/** A control-panel call; every one of its methods answers an object. */
export async function callControlPanel<T>(channel: ZoneChannel, method: string, params: Record<string, unknown> = {}): Promise<T> {
  const result = await channel.rpc<T>('control-panel', method, params)
  if (!result || typeof result !== 'object') throw new Error(`Invalid ${method} response`)
  return result
}
