/* ── How shared code reaches a Zone ──
 *
 * Code under `src/shared/` is used by the Shell and by the BuckyOS APP client (which imports this directory as
 * `@buckyos-desktop/*`). It imports only the `buckyos` package and files in here (relative, with `.ts`), and holds no
 * React, no build-time flags, no mock branches and no SDK global runtime: what it needs of a Zone goes through the
 * channel its caller passes, so one page can talk to several Zones. `tests/datamodel/shared-boundary.test.ts` checks it.
 */

import { MsgCenterClient, TaskManagerClient } from 'buckyos'

export interface ZoneChannel {
  /** Calls `method` of a Zone service (`https://<zone host>/kapi/<service>`); rejects when the call fails. */
  rpc<T>(service: string, method: string, params?: Record<string, unknown>): Promise<T>
  /** A same-origin request to the Zone's root (`/api/desktop`); the channel authenticates it. */
  fetch(path: string, init?: RequestInit): Promise<Response>
}

type RpcClient = ConstructorParameters<typeof MsgCenterClient>[0]

/** A service of the channel's Zone as the SDK's typed clients take it (they only use `call`). */
function serviceRpc(channel: ZoneChannel, service: string): RpcClient {
  return { call: (method: string, params: Record<string, unknown>) => channel.rpc(service, method, params) } as unknown as RpcClient
}

export function msgCenterOf(channel: ZoneChannel): MsgCenterClient {
  return new MsgCenterClient(serviceRpc(channel, 'msg-center'))
}

export function taskManagerOf(channel: ZoneChannel): TaskManagerClient {
  return new TaskManagerClient(serviceRpc(channel, 'task-manager'))
}
