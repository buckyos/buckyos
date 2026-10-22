/* ── Whom this browser session speaks for: an account, an External DID or nobody ── */

import { useCallback, useEffect, useState } from 'react'
import { ANONYMOUS_IDENTITY, buckyos, type CurrentIdentity } from 'buckyos'
import { isMockRuntime } from '../runtime'

/** The SDK identity (`/sso_refresh`), or the mock control panel's session in the mock runtime. */
export async function loadCurrentIdentity(): Promise<CurrentIdentity> {
  if (isMockRuntime()) {
    const { mockCurrentIdentity } = await import('../api/control_panel_mock.ts')
    return mockCurrentIdentity()
  }
  return buckyos.getCurrentIdentity()
}

let bootIdentity: CurrentIdentity = ANONYMOUS_IDENTITY

/** Set once by `bootstrap()` before the first render. */
export function setBootIdentity(identity: CurrentIdentity) {
  bootIdentity = identity
}

export function getBootIdentity(): CurrentIdentity {
  return bootIdentity
}

/** Limited accounts and External DIDs: they get the restricted shell, never the full Desktop. */
export function isRestrictedIdentity(identity: CurrentIdentity | null | undefined): boolean {
  return !!identity && identity.kind !== 'anonymous' && identity.restricted
}

export function identityDisplayName(identity: CurrentIdentity): string {
  switch (identity.kind) {
    case 'account':
      return identity.show_name || identity.user_id
    case 'external':
      return identity.show_name || identity.did
    default:
      return ''
  }
}

/** A Limited account that has not set a password yet (it entered through an exclusive invite). */
export function needsFirstPassword(identity: CurrentIdentity | null | undefined): boolean {
  return !!identity && identity.kind === 'account' && identity.restricted && identity.password_set === false
}

/** The current identity; `null` while loading. `reload` reads it again (after a sign-out, say). */
export function useCurrentIdentity() {
  const [identity, setIdentity] = useState<CurrentIdentity | null>(null)
  const [seq, setSeq] = useState(0)
  useEffect(() => {
    let live = true
    void loadCurrentIdentity()
      .catch(() => ANONYMOUS_IDENTITY)
      .then((next) => { if (live) setIdentity(next) })
    return () => { live = false }
  }, [seq])
  const reload = useCallback(() => setSeq((value) => value + 1), [])
  return { identity, reload }
}
