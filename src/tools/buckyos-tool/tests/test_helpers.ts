import type { ResolvedConfig } from '../core/config.ts'

export function assert(condition: unknown, message = 'assertion failed'): asserts condition {
  if (!condition) throw new Error(message)
}

export function assertEquals(actual: unknown, expected: unknown, message?: string): void {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      message ?? `expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
    )
  }
}

export async function assertRejects(
  run: () => Promise<unknown> | unknown,
  expectedCode?: string,
): Promise<void> {
  try {
    await run()
  } catch (error) {
    if (expectedCode) {
      const code = (error as { code?: unknown }).code
      assertEquals(code, expectedCode)
    }
    return
  }
  throw new Error('expected operation to reject')
}

export function jwt(claims: Record<string, unknown>): string {
  return `${base64Url(JSON.stringify({ alg: 'none' }))}.${
    base64Url(JSON.stringify(claims))
  }.signature`
}

export function testConfig(overrides: Partial<ResolvedConfig> = {}): ResolvedConfig {
  return {
    configDir: '/tmp/buckyos-tool-test',
    zone: 'test.example.com',
    endpoint: 'https://test.example.com',
    output: 'json',
    defaultProtocol: 'https://',
    timeoutMs: 30_000,
    wait: false,
    nonInteractive: true,
    yes: false,
    noColor: true,
    verbose: false,
    sources: {},
    ...overrides,
  }
}

function base64Url(value: string): string {
  const bytes = new TextEncoder().encode(value)
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '')
}
