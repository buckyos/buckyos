/**
 * "My files": the signed-in user's own home folder, resolved by the backend
 * registered in install.ts — the NFSP hello identity (`/home/<user_id>`), or
 * `/home` where there is no user (no auth, mock fixtures). Components use the
 * hook; the first resolution is cached so later windows mount without waiting.
 */

import { useEffect, useState } from 'react'

export interface MyHome {
  path: string
  userId: string | null
}

const FALLBACK: MyHome = { path: '/home', userId: null }

let provider: (() => Promise<MyHome>) | null = null
let resolved: MyHome | null = null
let pending: Promise<MyHome> | null = null

export function registerMyHomeProvider(next: () => Promise<MyHome>): () => void {
  provider = next
  resolved = null
  pending = null
  return () => {
    if (provider === next) provider = null
  }
}

export function myHome(): Promise<MyHome> {
  if (resolved) return Promise.resolve(resolved)
  if (!pending) {
    const attempt = (provider ? provider() : Promise.resolve(FALLBACK))
      .catch(() => FALLBACK)
      .then((home) => {
        if (pending === attempt) resolved = home
        return home
      })
    pending = attempt
  }
  return pending
}

/** The resolved home, or null on the very first resolution. */
export function useMyHome(): MyHome | null {
  const [home, setHome] = useState<MyHome | null>(resolved)
  useEffect(() => {
    if (home) return
    let live = true
    void myHome().then((value) => { if (live) setHome(value) })
    return () => { live = false }
  }, [home])
  return home
}
