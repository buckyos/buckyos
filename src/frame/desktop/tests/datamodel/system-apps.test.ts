/**
 * The addresses of the system apps' own pages, their pages and the launches those stand for, and the list of apps.
 *   deno test --no-check -A --node-modules-dir=manual --unstable-sloppy-imports tests/datamodel/system-apps.test.ts
 */
import { readAppWindowDeclaration } from 'buckyos/open-app'
import { appPath, systemAppAddress, systemAppPath } from '../../src/app/standalone/address.ts'
import { hasStandalonePage, systemApps } from '../../src/app/standalone/catalog.ts'
import { windowDeclarationOf, windowManifestOf } from '../../src/app/windowDeclaration.ts'

function assertEquals<T>(actual: T, expected: T, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
}

const sorted = (value: object) => Object.fromEntries(Object.entries(value).sort(([a], [b]) => (a < b ? -1 : 1)))

const at = (address: string) => {
  const url = new URL(address, 'https://zone/')
  return { pathname: url.pathname, search: url.search, hash: url.hash }
}

Deno.test('a page of an app is its address with the path after it, and reads back', () => {
  const cases: Array<[string, string | undefined, string]> = [
    ['settings', undefined, '/app/settings'],
    ['settings', '/', '/app/settings'],
    ['settings', '/appearance', '/app/settings/appearance'],
    ['ai-center', '/providers/add', '/app/ai-center/providers/add'],
    ['files', '/?path=%2Fhome', '/app/files?path=%2Fhome'],
    ['my-network', '/add-friend?input=did%3Aweb%3Ax', '/app/my-network/add-friend?input=did%3Aweb%3Ax'],
    ['task-center', '/?taskid=1', '/taskcenter?taskid=1'],
    ['messagehub', '/?notify=n1', '/messagehub?notify=n1'],
    ['homestation', '/alice/post', '/homestation/alice/post'],
    ['aiworkspace', '/', '/workspace'],
  ]
  for (const [app, path, address] of cases) {
    assertEquals(systemAppAddress(app, path), address, `${app} ${path}`)
    assertEquals(systemAppPath(app, at(address)), path ?? '/', `${app} ${address}`)
  }
  assertEquals(appPath('', { a: undefined, b: null, c: '' }), '/', 'no parameters')
  assertEquals(appPath('x', { path: '/home/a b' }), '/x?path=%2Fhome%2Fa+b', 'parameters are encoded')
})

Deno.test('the system apps are the desktop built-ins that mean something without a desktop', () => {
  const ids = systemApps().map((app) => app.id)
  for (const id of ['files', 'settings', 'messagehub', 'preview', 'agent-guide']) assertEquals(ids.includes(id), true, id)
  for (const id of ['diagnostics', 'demos', 'market', 'studio', 'docs', 'codeassistant', 'systest']) assertEquals(hasStandalonePage(id), false, id)
})

Deno.test('a window declaration survives the desktop manifest and back', () => {
  for (const app of systemApps()) {
    const declaration = windowDeclarationOf(app.manifest)
    assertEquals(sorted(readAppWindowDeclaration(declaration)), sorted(declaration), `${app.id} reads back`)
    assertEquals(windowDeclarationOf({ ...app.manifest, ...windowManifestOf(declaration) }), declaration, `${app.id} round trip`)
  }
})
