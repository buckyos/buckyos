/* Where the reflow library and its pages are: on a Zone, the user's bucket and the host `reflow.<zone>` (Agent
 * Browser Protocol §5.1, §5.4); in the mock runtime, mock/reflow.ts. Everything above this file is the same code
 * in both. */

import { bucket, buckyos } from 'buckyos'
import { fetchCurrentAccount } from '../../../api/account'
import { requestServiceLaunch } from '../../../api/app_launch'
import { isMockRuntime } from '../../../runtime'
import { REFLOW_APP, REFLOW_BUCKET, type Library } from './library'

/** What the frame loads for a page, or why the user could not be signed in on the pages' origin. */
export type PageLoad =
  | { kind: 'ready'; src: string }
  | { kind: 'ready'; srcdoc: string }
  | { kind: 'access_denied' | 'invalid_target' | 'session_inactive' | 'failed' }

export interface ReflowSource {
  library(): Promise<Library>
  /** The origin reflow pages are served from: the only one whose messages the App takes (§5.5). */
  readonly origin: string
  /** Signs the user in on that origin for `view` (a path of §5.3); each load of the frame goes through it again. */
  openPage(view: string): Promise<PageLoad>
}

const SERVICE = 'nfs-server'

const zoneSource: ReflowSource = {
  async library() {
    const account = await fetchCurrentAccount()
    if (!account) throw Error('not signed in')
    return { store: buckyos.getBucketClient(), bucketId: bucket.userBucketId(REFLOW_APP, account.user_id, REFLOW_BUCKET), userId: account.user_id }
  },
  get origin() {
    const zone = buckyos.getZoneHostName()?.trim()
    return zone ? `${window.location.protocol}//reflow.${zone}${window.location.port ? `:${window.location.port}` : ''}` : ''
  },
  async openPage(view) {
    if (!this.origin) return { kind: 'failed' }
    const launch = await requestServiceLaunch(SERVICE, `${this.origin}/${view}`).catch(() => null)
    switch (launch?.status) {
      case 'ready': return { kind: 'ready', src: launch.callback_url }
      case 'access_denied': return { kind: 'access_denied' }
      case 'invalid_target': return { kind: 'invalid_target' }
      case 'source_session_inactive': return { kind: 'session_inactive' }
      default: return { kind: 'failed' }
    }
  },
}

let source: Promise<ReflowSource> | null = null

export function getReflowSource(): Promise<ReflowSource> {
  source ??= isMockRuntime() ? import('../mock/reflow').then((module) => module.mockReflowSource) : Promise.resolve(zoneSource)
  return source
}
