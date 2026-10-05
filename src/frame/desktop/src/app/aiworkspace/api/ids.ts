/* Caller-generated identifiers (design §2.3): `^[a-z0-9][a-z0-9_-]{0,63}$`. */

const BASE32 = 'abcdefghijklmnopqrstuvwxyz234567'

/** 20 lowercase base32 characters (100 random bits). */
export function randomId(prefix = ''): string {
  const bytes = new Uint8Array(20)
  crypto.getRandomValues(bytes)
  let out = ''
  for (const byte of bytes) out += BASE32[byte & 31]
  return prefix + out
}

export const ID_PATTERN = /^[a-z0-9][a-z0-9_-]{0,63}$/

export function bytesToBase64(bytes: Uint8Array): string {
  let binary = ''
  const chunk = 0x8000
  for (let i = 0; i < bytes.length; i += chunk) binary += String.fromCharCode(...bytes.subarray(i, i + chunk))
  return btoa(binary)
}

export function base64ToBytes(text: string): Uint8Array {
  const binary = atob(text)
  const out = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i)
  return out
}
