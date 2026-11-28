import { useEffect, useMemo, useState, useSyncExternalStore } from 'react'
import { createPolicyApi, type PolicyApi } from '../../../api/aicc_policy'
import { useAICCStore } from './use-aicc-store'
import type { EffectiveModel, PolicyEffectiveResponse, PolicyStatusResponse } from '../datamodel/policy'

export function usePolicyApi(): PolicyApi {
  const store = useAICCStore()
  return useMemo(() => createPolicyApi(store), [store])
}

/** The viewer's own effective policy (prices, usability, limits, balance), for marks outside the Policies page.
 * Reloads when the AICC snapshot changes (a Provider added or refreshed). */
export function useMyEffective(): PolicyEffectiveResponse | null {
  const api = usePolicyApi()
  const store = useAICCStore()
  const version = useSyncExternalStore(store.subscribe, store.getSnapshotVersion)
  const [effective, setEffective] = useState<PolicyEffectiveResponse | null>(null)
  useEffect(() => {
    let live = true
    api.effective().then((result) => { if (live) setEffective(result) }).catch(() => undefined)
    return () => { live = false }
  }, [api, version])
  return effective
}

/** Effective models of the viewer by exact model. */
export function useMyModelPrices(): { effective: PolicyEffectiveResponse | null; models: Map<string, EffectiveModel> } {
  const effective = useMyEffective()
  const models = useMemo(() => new Map((effective?.models ?? []).map((model) => [model.exact_model, model])), [effective])
  return { effective, models }
}

/** The viewer's balance and limit states (`policy.status`). */
export function usePolicyStatus(): PolicyStatusResponse | null {
  const api = usePolicyApi()
  const [status, setStatus] = useState<PolicyStatusResponse | null>(null)
  useEffect(() => {
    let live = true
    api.status().then((result) => { if (live) setStatus(result) }).catch(() => undefined)
    return () => { live = false }
  }, [api])
  return status
}
