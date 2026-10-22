import { useEffect, useMemo, useState } from 'react'
import { createQuotaApi, type QuotaApi } from '../../../api/aicc_quota'
import { useAICCStore } from './use-aicc-store'
import type { QuotaRuleView } from '../datamodel/quota'

export function useQuotaApi(): QuotaApi {
  const store = useAICCStore()
  return useMemo(() => createQuotaApi(store), [store])
}

/** The rules that apply to the viewer (own and administrator), for status marks outside the Quota page. */
export function useMyQuotaRules(): QuotaRuleView[] {
  const api = useQuotaApi()
  const [rules, setRules] = useState<QuotaRuleView[]>([])
  useEffect(() => {
    let live = true
    api.query().then((result) => { if (live) setRules(result.rules) }).catch(() => undefined)
    return () => { live = false }
  }, [api])
  return rules
}
