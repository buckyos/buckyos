import { useCallback, useState } from 'react'
import type { ApplyResult, CertificateSource, DomainPlan, DomainStatus, HostingMode } from '../../../../api/zone_domain'
import type { CheckedCertificate } from './labels'

/** `hosting` changes how the domain is managed; the other two replace one thing and keep the rest. */
export type EditorIntent = 'hosting' | 'token' | 'certificate'

export interface DomainDraft {
  intent: EditorIntent
  step: 'choose' | 'setup'
  mode: HostingMode
  certificate: CertificateSource
  plan: DomainPlan | null
  /** What the node wrote and the credential it holds for it. */
  applied: ApplyResult | null
  keepToken: boolean
  keepCertificate: boolean
  uploaded: CheckedCertificate | null
}

export function newDraft(intent: EditorIntent, status: DomainStatus): DomainDraft {
  const hosting = status.hosting!
  const replacing = intent !== 'hosting'
  return {
    intent,
    step: replacing ? 'setup' : 'choose',
    mode: hosting.mode,
    certificate: hosting.certificate,
    plan: null,
    applied: null,
    keepToken: !replacing && Boolean(status.credential) && !status.problems.some((problem) => problem.startsWith('credential_')),
    keepCertificate: !replacing && hosting.certificate === 'upload',
    uploaded: null,
  }
}

/* The change survives leaving the page for the DNS provider's site. It holds no token and no key, only the ids of what
 * the node keeps. */
let saved: DomainDraft | null = null

export interface DomainDraftState {
  draft: DomainDraft | null
  start: (draft: DomainDraft) => void
  patch: (change: Partial<DomainDraft>) => void
  clear: () => void
}

export function useDomainDraft(): DomainDraftState {
  const [draft, setDraft] = useState(saved)
  const store = useCallback((next: DomainDraft | null) => {
    saved = next
    setDraft(next)
  }, [])
  return {
    draft,
    start: store,
    patch: useCallback((change) => store(saved && { ...saved, ...change }), [store]),
    clear: useCallback(() => store(null), [store]),
  }
}
