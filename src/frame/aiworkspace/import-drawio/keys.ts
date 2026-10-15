/* Identifiers and sibling order keys the importer writes, by the core's rules (`core/src/order_key.rs`, ids in
 * design §2.3): ids `^[a-z0-9][a-z0-9_-]{0,63}$`; order keys base-36, compared bytewise, never ending in `0`. */

const BASE32 = 'abcdefghijklmnopqrstuvwxyz234567'

/** 20 lowercase base32 characters (100 random bits) after `prefix`. */
export function randomId(prefix = ''): string {
  const bytes = new Uint8Array(20)
  crypto.getRandomValues(bytes)
  let out = ''
  for (const byte of bytes) out += BASE32[byte & 31]
  return prefix + out
}

const DIGITS = '0123456789abcdefghijklmnopqrstuvwxyz'
const idx = (c: string | undefined) => (c === undefined ? 0 : Math.max(0, DIGITS.indexOf(c)))

function midpoint(a: string, b: string | null): string {
  if (b !== null) {
    let n = 0
    while (n < b.length && (a[n] ?? '0') === b[n]) n += 1
    if (n > 0) return b.slice(0, n) + midpoint(a.slice(n), b.slice(n))
  }
  const da = idx(a[0])
  const db = b !== null ? idx(b[0]) : DIGITS.length
  if (db - da > 1) return DIGITS[Math.floor((da + db + 1) / 2)]
  if (b !== null && b.length > 1) return b[0]
  return DIGITS[da] + midpoint(a.slice(1), null)
}

/** A key strictly between `a` and `b` (null: open end); `a < b` required. The same algorithm as the core. */
export function keyBetween(a: string | null, b: string | null): string {
  return midpoint(a ?? '', b)
}

/** `n` increasing keys after `after` (null: from the start), all of one length: a fresh key after `after`
 * followed by evenly spread fixed-width suffixes, so thousands of siblings stay far below the 64-character limit. */
export function keysAfter(after: string | null, n: number): string[] {
  if (n <= 0) return []
  const head = keyBetween(after, null)
  if (n === 1) return [head]
  let width = 1
  while (36 ** width - 36 ** (width - 1) < n * 2) width += 1
  const span = 36 ** width
  const out: string[] = []
  for (let i = 0; i < n; i++) {
    let v = Math.floor(((i + 1) * span) / (n + 1))
    if (v % 36 === 0) v += 1
    out.push(head + v.toString(36).padStart(width, '0'))
  }
  return out
}
