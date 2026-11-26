import { presentApp, resolveImageReference } from '../../src/presentation/model.ts'

function assertEquals(actual: unknown, expected: unknown) {
  if (actual !== expected) throw new Error(`Expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
}

const appBase = 'https://photos.home.example/gallery/'

Deno.test('root-relative app icons use the app origin and preserve the version query', () => {
  assertEquals(resolveImageReference('/icon.svg?v=0.1.3', appBase), 'https://photos.home.example/icon.svg?v=0.1.3')
})

Deno.test('relative app icons use the supplied base path', () => {
  assertEquals(resolveImageReference('res/icon.svg', appBase), 'https://photos.home.example/gallery/res/icon.svg')
  assertEquals(resolveImageReference('./icon.svg', appBase), 'https://photos.home.example/gallery/icon.svg')
})

Deno.test('absolute and image data icons retain their own origin', () => {
  assertEquals(resolveImageReference('https://cdn.example/icon.svg', appBase), 'https://cdn.example/icon.svg')
  assertEquals(resolveImageReference('data:image/svg+xml,%3Csvg/%3E', appBase), 'data:image/svg+xml,%3Csvg/%3E')
})

Deno.test('root-relative system icons stay same-origin when no app base is supplied', () => {
  assertEquals(resolveImageReference('/favicon.svg'), '/favicon.svg')
  assertEquals(resolveImageReference('icon.svg'), undefined)
})

Deno.test('invalid icon values and unsupported schemes are rejected', () => {
  for (const value of [null, '', ' ', 123, 'javascript:alert(1)', 'data:text/html,hello', 'file:///icon.svg']) {
    assertEquals(resolveImageReference(value, appBase), undefined)
  }
  assertEquals(resolveImageReference('/icon.svg', 'not a URL'), undefined)
})

Deno.test('app presentation exposes a loadable URL for a root-relative icon', () => {
  const app = presentApp({key: 'photos@alice', showName: 'Photos', iconUrl: '/icon.svg?v=2', iconBase: appBase}, 'en')
  assertEquals(app.icons[0], 'https://photos.home.example/icon.svg?v=2')
})
