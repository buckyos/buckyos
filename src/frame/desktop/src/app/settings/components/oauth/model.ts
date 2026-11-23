import type { OAuthSource } from '../../../../api/sso'

export type SourceKind = OAuthSource['kind']
type Translate = (key: string, fallback?: string, variables?: Record<string, string | number>) => string

export type Field =
  | 'id'
  | 'display_name'
  | 'client_id'
  | 'secret_ref'
  | 'webhost'
  | 'issuer'
  | 'authorization_endpoint'
  | 'token_endpoint'
  | 'jwks_uri'
  | 'account_endpoint'
  | 'id_field'
  | 'email_field'
  | 'email_verified_field'

export const FIELD_LABELS: Record<Field, string> = {
  id: 'Source ID',
  display_name: 'Display name',
  client_id: 'Client ID',
  secret_ref: 'Secret file name',
  webhost: 'Identity domain',
  issuer: 'Issuer URL',
  authorization_endpoint: 'Authorization endpoint',
  token_endpoint: 'Token endpoint',
  jwks_uri: 'JWKS URL',
  account_endpoint: 'Account endpoint',
  id_field: 'Stable account ID field',
  email_field: 'Email field',
  email_verified_field: 'Verified-email field',
}

export const KIND_LABELS: Record<SourceKind, string> = {
  google: 'Google',
  oidc: 'OpenID Connect',
  oauth2: 'OAuth 2.0',
}

/** The fields an admin fills in, in form order. Google's endpoints are preset by the service. */
export function formFields(kind: SourceKind): Field[] {
  if (kind === 'google') return ['display_name', 'client_id']
  if (kind === 'oidc') {
    return ['id', 'display_name', 'client_id', 'secret_ref', 'webhost', 'issuer', 'authorization_endpoint', 'token_endpoint', 'jwks_uri']
  }
  return ['id', 'display_name', 'client_id', 'secret_ref', 'webhost', 'authorization_endpoint', 'token_endpoint', 'account_endpoint', 'id_field', 'email_field', 'email_verified_field']
}

/**
 * What names a source's accounts, so the service refuses to change it once saved:
 * the ID, the identity domain and the issuer (an OAuth 2.0 source's issuer is its account endpoint).
 */
export function lockedFields(kind: SourceKind): Set<Field> {
  return new Set<Field>(kind === 'oauth2' ? ['id', 'webhost', 'account_endpoint'] : ['id', 'webhost', 'issuer'])
}

const URL_FIELDS = new Set<Field>(['issuer', 'authorization_endpoint', 'token_endpoint', 'jwks_uri', 'account_endpoint'])
const SOURCE_ID = /^[a-z0-9_-]{1,64}$/

export function newSource(kind: SourceKind): OAuthSource {
  if (kind === 'google') {
    return { kind, id: 'google', display_name: 'Google', enabled: true, client_id: '', secret_ref: 'google-demo' }
  }
  return { kind, id: '', display_name: '', enabled: true, client_id: '', secret_ref: '', scopes: [] }
}

/** The first problem with a field, or null; empty fields count once the admin has seen them. */
export function fieldError(source: OAuthSource, field: Field, t: Translate): string | null {
  const value = String(source[field] ?? '').trim()
  if (!value) return t('sso.provider.required', 'Required')
  if ((field === 'id' || field === 'secret_ref') && !SOURCE_ID.test(value)) {
    return t('sso.provider.idFormat', 'Lowercase letters, digits, - and _ only')
  }
  if (field === 'id' && value === 'google') return t('sso.provider.googleReserved', '“google” is kept for the Google preset')
  if (URL_FIELDS.has(field) && !/^https:\/\/[^/\s]+/.test(value)) return t('sso.provider.httpsOnly', 'Use an https:// address')
  if (field === 'webhost' && (!value.includes('.') || /[/:\s]/.test(value))) {
    return t('sso.provider.domainFormat', 'A domain name such as example.com')
  }
  return null
}

export function sourceComplete(source: OAuthSource, t: Translate): boolean {
  return formFields(source.kind).every((field) => fieldError(source, field, t) === null)
}

export function callbackUrl(origin: string, id: string): string {
  return new URL('/sso/oauth/' + encodeURIComponent(id) + '/callback', origin).href
}

/** The service's refusals that an admin can act on, in their words. */
export function saveErrorText(error: unknown, t: Translate): string {
  const detail = error instanceof Error ? error.message : String(error ?? '')
  if (detail.includes('oauth_source_retired')) {
    return t('sso.provider.errorRetired', 'This source ID, identity domain or issuer belonged to a deleted login source. Only that same source can be added again.')
  }
  if (detail.includes('identity domain must contain')) {
    return t('sso.provider.errorDomain', 'The identity domain must contain the host of the issuer or account endpoint.')
  }
  if (detail.includes('duplicate OAuth identity domain or issuer')) {
    return t('sso.provider.errorDuplicateDomain', 'Another source already uses this identity domain or issuer, or the domain is inside this Zone.')
  }
  if (detail.includes('duplicate OAuth source ID')) {
    return t('sso.provider.errorDuplicateId', 'Another source already uses this source ID.')
  }
  if (detail.includes('Google identity namespace is reserved')) {
    return t('sso.provider.errorGoogleReserved', 'google and google.com are kept for the Google preset.')
  }
  return t('sso.provider.saveFailed', 'Not saved: {{detail}}', { detail })
}
