import type { CertificateInspection, CheckStatus, DnsProvider, DomainCheckItem, DomainHosting, HostingMode, TimeValue } from '../../../../api/zone_domain'
import { domainErrorOf } from '../../../../api/zone_domain'
import type { useI18n } from '../../../../i18n/provider'

export type Translate = ReturnType<typeof useI18n>['t']

export type Tone = 'pass' | 'warn' | 'fail' | 'neutral'

export const TONE_COLORS: Record<Tone, string> = {
  pass: 'var(--cp-success)',
  warn: 'var(--cp-warning)',
  fail: 'var(--cp-danger)',
  neutral: 'var(--cp-muted)',
}

export const CHECK_TONES: Record<CheckStatus, Tone> = { pass: 'pass', waiting: 'warn', fail: 'fail', skipped: 'neutral' }

export const MODES: HostingMode[] = ['sn', 'dns_api', 'manual']

/** What a check or record code means; the SN's refusals (`bind_…`) share one text unless they have their own. */
export function codeText(t: Translate, code: string | null | undefined): string | null {
  if (!code) return null
  const own = t(`settings.domain.code.${code}`, '')
  if (own) return own
  if (code.startsWith('bind_')) return t('settings.domain.code.bind', undefined, { code })
  return t('settings.domain.code.unknown', undefined, { code })
}

/** Not a message: the TXT record is hidden because the domain is already delegated to the SN. */
const DELEGATED = 'delegated_to_sn'

/** What a check item says, and its raw message. */
export function itemText(t: Translate, item: DomainCheckItem): { text: string | null; detail: string | null } {
  return item.detail === DELEGATED
    ? { text: codeText(t, DELEGATED), detail: null }
    : { text: codeText(t, item.code), detail: item.detail }
}

export interface DomainFailure {
  text: string
  /** The raw message, only for an error without a text of its own. */
  detail: string | null
}

export function failureOf(t: Translate, error: unknown): DomainFailure {
  const { code, detail } = domainErrorOf(error)
  if (code) {
    const own = t(`settings.domain.error.${code}`, '') || t(`settings.domain.code.${code}`, '')
    if (own) return { text: own, detail: null }
    if (code.startsWith('bind_')) return { text: t('settings.domain.code.bind', undefined, { code }), detail: null }
  }
  return { text: t('settings.domain.error.other'), detail: detail || null }
}

export function problemText(t: Translate, code: string): string {
  return t(`settings.domain.problem.${code}`, '') || t('settings.domain.problem.other', undefined, { code })
}

export function certificateCheckLabel(t: Translate, code: string, domain: string): string {
  return t(`settings.domain.check.${code}`, code, { domain })
}

export function certificateReasonText(t: Translate, reason: string | null, domain: string): string | null {
  if (!reason) return null
  return t(`settings.domain.reason.${reason}`, '', { domain }) || t('settings.domain.code.unknown', undefined, { code: reason })
}

/** Only clients that trust it can connect: a self-signed certificate, or one from a CA that is not public. */
export function isPrivateCertificate(inspection: CertificateInspection): boolean {
  return inspection.items.some((item) => item.code === 'public_trust' && item.level === 'warn')
}

export function hasCertificateWarning(inspection: CertificateInspection): boolean {
  return inspection.items.some((item) => item.level === 'warn')
}

export function certificateSourceText(t: Translate, hosting: DomainHosting): string {
  return t(`settings.domain.source.${hosting.certificate}`)
}

export function toDate(value: TimeValue | undefined): Date | null {
  if (value == null || value === '') return null
  const date = new Date(typeof value === 'number' ? value * 1000 : value)
  return Number.isNaN(date.getTime()) ? null : date
}

/** A certificate that passed the node's checks; the node holds it until it is saved. */
export interface CheckedCertificate {
  id: string
  inspection: CertificateInspection
  /** The user agreed to the warnings. */
  accepted: boolean
}

export const certificateUsable = (certificate: CheckedCertificate | null) =>
  Boolean(certificate && (certificate.accepted || !hasCertificateWarning(certificate.inspection)))

export function providerFieldsComplete(provider: DnsProvider | null, fields: Record<string, string>): boolean {
  return Boolean(provider && provider.fields.every((field) => field.optional || fields[field.id]?.trim()))
}

/** The filled-in fields of `provider`, without the empty optional ones. */
export function providerFieldValues(provider: DnsProvider, fields: Record<string, string>): Record<string, string> {
  return Object.fromEntries(provider.fields.flatMap((field) => {
    const value = fields[field.id]?.trim()
    return value ? [[field.id, value]] : []
  }))
}
