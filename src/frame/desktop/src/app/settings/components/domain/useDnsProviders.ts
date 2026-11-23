import { useEffect, useState } from 'react'
import { fetchDnsProviders, type DnsProvider } from '../../../../api/zone_domain'

interface ProvidersState {
  providers: DnsProvider[] | null
  error: unknown
}

/** The DNS services the node can write to, read when `enabled`. */
export function useDnsProviders(enabled: boolean): ProvidersState {
  const [state, setState] = useState<ProvidersState>({ providers: null, error: null })
  useEffect(() => {
    if (!enabled) return
    let live = true
    void fetchDnsProviders().then(({ data, error }) => {
      if (live) setState({ providers: data?.providers ?? null, error: data ? null : error })
    })
    return () => { live = false }
  }, [enabled])
  return state
}
