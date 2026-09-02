/**
 * Shared NfsBrowserClient instance for the File Browser NFSP adapter.
 *
 * In buckyos mode the gateway forwards the zone-level root path `/nfs/v1/*`
 * straight to nfs-server (no /kapi prefix), so the base URL is the page
 * origin. `?fbServer=<url>` overrides it for diagnosing against a standalone
 * nfs_server (e.g. http://127.0.0.1:3260).
 *
 * `ensureSession()` performs the initial hello exactly once (deduped);
 * session expiry replay is handled inside NfsBrowserClient.
 */

import { NfsBrowserClient } from '../../../../api/nfs_browser_client'
import type { HelloResult } from '../../../../api/nfsp_client'

let client: NfsBrowserClient | null = null
let helloPromise: Promise<HelloResult> | null = null

export function nfspBaseUrl(): string {
  try {
    const override = new URLSearchParams(window.location.search).get('fbServer')
    if (override) return override.replace(/\/+$/, '')
  } catch {
    // no window/search — fall through to the origin
  }
  return window.location.origin
}

export function nfspClient(): NfsBrowserClient {
  if (!client) {
    client = new NfsBrowserClient({ baseUrl: nfspBaseUrl() })
  }
  return client
}

/** Hello once per app lifetime; concurrent callers share the same promise. */
export function ensureSession(): Promise<HelloResult> {
  if (helloPromise) return helloPromise
  const attempt: Promise<HelloResult> = nfspClient()
    .hello()
    .catch((err: unknown) => {
      // A failed hello must not poison every later call: allow a retry.
      if (helloPromise === attempt) helloPromise = null
      throw err
    })
  helloPromise = attempt
  return attempt
}

/** Effective list page size: min(UI page, server max_list); pre-hello 200. */
export function effectiveListLimit(requested: number): number {
  const max = nfspClient().raw.limits?.max_list
  return max ? Math.min(requested, max) : requested
}
