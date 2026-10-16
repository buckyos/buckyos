/* A guest of a public link (UI improvement §8.3): the service names it `guest:<26 chars>` after the token this
 * browser keeps; what people see is "访客 <6 chars>" or the name the guest chose here, which annotations carry
 * as `author_name` (the principal stays `author`). */

import type { AnnotationPayload } from '../api/types'
import { GUEST_PREFIX, isGuestPrincipal } from '../api/transport'
import { aiwsT } from '../i18n'

export const GUEST_NAME_KEY = 'aiworkspace.guest-name'

export function readGuestName(): string {
  try { return (window.localStorage.getItem(GUEST_NAME_KEY) ?? '').trim().slice(0, 32) } catch { return '' }
}

export function writeGuestName(name: string) {
  try {
    const trimmed = name.trim().slice(0, 32)
    if (trimmed) window.localStorage.setItem(GUEST_NAME_KEY, trimmed)
    else window.localStorage.removeItem(GUEST_NAME_KEY)
  } catch { /* storage unavailable: the name lasts for this page */ }
}

/** "访客 abc123" for a guest principal. */
export function guestLabel(principal: string): string {
  return aiwsT('aiworkspace.core.guest.label', { id: principal.slice(GUEST_PREFIX.length, GUEST_PREFIX.length + 6) })
}

/** The author line of an annotation: the chosen name of a guest, else its short label, else the principal. */
export function authorLabel(payload: Pick<AnnotationPayload, 'author' | 'author_name'>): string | null {
  const name = payload.author_name?.trim()
  if (name) return name
  if (!payload.author) return null
  return isGuestPrincipal(payload.author) ? guestLabel(payload.author) : payload.author
}
