import { useCallback, useEffect, useState } from 'react'
import { verifyDomain, type DomainCheck, type DomainSetup, type SetupIds } from '../../../../api/zone_domain'
import type { DomainSudo } from './useDomainSudo'

const INTERVAL_MS = 10_000

interface VerifyState {
  /** The setup this result is for. */
  key: string
  check: DomainCheck | null
  error: unknown
  /** The permission ran out: checking goes on after the user confirms again. */
  expired: boolean
}

export interface Verify extends Omit<VerifyState, 'key'> {
  busy: boolean
  checkNow: () => Promise<void>
}

/** Verifies a setup every 10 seconds until everything passes; a changed setup starts over. */
export function useVerify(enabled: boolean, setup: DomainSetup, ids: SetupIds, sudo: DomainSudo): Verify {
  const { mode, certificate } = setup
  const { credential_id: credentialId, certificate_id: certificateId } = ids
  const key = [mode, certificate, credentialId, certificateId].join('|')
  const [state, setState] = useState<VerifyState | null>(null)
  const [busy, setBusy] = useState(false)
  const [round, setRound] = useState(0)
  const { acquire, current } = sudo

  useEffect(() => {
    if (!enabled) return
    let stopped = false
    let timer = 0
    const next = () => { timer = window.setTimeout(() => void tick(), INTERVAL_MS) }
    const tick = async () => {
      if (document.visibilityState === 'hidden') return next()
      const options = current()
      if (!options) {
        setState((before) => ({ key, check: before?.key === key ? before.check : null, error: null, expired: true }))
        return
      }
      setBusy(true)
      const { data, error } = await verifyDomain(
        { mode, certificate },
        { ...(credentialId ? { credential_id: credentialId } : {}), ...(certificateId ? { certificate_id: certificateId } : {}) },
        options,
      )
      if (stopped) return
      setBusy(false)
      setState((before) => ({ key, check: data ?? (before?.key === key ? before.check : null), error: data ? null : error, expired: false }))
      if (!data?.ready) next()
    }
    timer = window.setTimeout(() => void tick(), 0)
    return () => {
      stopped = true
      window.clearTimeout(timer)
      setBusy(false)
    }
  }, [enabled, key, round, mode, certificate, credentialId, certificateId, current])

  const checkNow = useCallback(async () => {
    if (await acquire()) setRound((value) => value + 1)
  }, [acquire])

  const own = state?.key === key ? state : null
  return { check: own?.check ?? null, error: own?.error ?? null, expired: own?.expired ?? false, busy, checkNow }
}
