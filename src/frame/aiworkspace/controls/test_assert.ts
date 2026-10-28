// Minimal assertions for `deno test` without remote imports (the service runs Deno offline).
export function assert(cond: unknown, msg = 'assertion failed'): asserts cond {
  if (!cond) throw new Error(msg)
}

export function assertEquals(actual: unknown, expected: unknown, msg = '') {
  const a = JSON.stringify(actual)
  const e = JSON.stringify(expected)
  if (a !== e) throw new Error(`${msg ? msg + ': ' : ''}expected ${e}, got ${a}`)
}
