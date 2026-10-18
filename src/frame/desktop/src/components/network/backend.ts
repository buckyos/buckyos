/* ── Backend registry: mock world in the mock runtime, msg-center otherwise (same pattern as preview/provider.ts) ── */

import { isMockRuntime } from '../../runtime'
import type { NetworkBackend } from './types'

let backend: NetworkBackend | null = null
let installing: Promise<NetworkBackend> | null = null

export function ensureNetworkBackend(): Promise<NetworkBackend> {
  if (backend) return Promise.resolve(backend)
  installing ??= (isMockRuntime()
    ? import('./mock/backend').then((m) => m.createMockNetworkBackend())
    : import('./realBackend').then((m) => m.createRealNetworkBackend())
  ).then((created) => {
    backend ??= created
    installing = null
    return backend
  })
  return installing
}
