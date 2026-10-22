/** Channels an exclusive invite is sent through; the issuer sends it, the system only records the kind. */
export const INVITE_CHANNEL_KINDS = ['email', 'sms', 'other'] as const

/**
 * The masked hint of an address that is stored with an invite (`a***@acme.com`, `138****5678`);
 * the full address never leaves the issuer.
 */
export function maskChannelTarget(kind: string, value: string): string {
  const text = value.trim()
  if (!text) return ''
  const at = text.lastIndexOf('@')
  if (kind === 'email' && at > 0) return `${text.slice(0, 1)}***${text.slice(at)}`
  const compact = text.replace(/[\s-]+/g, '')
  if (compact.length >= 7) return `${compact.slice(0, 3)}****${compact.slice(-4)}`
  return `${text.slice(0, 1)}***`
}
