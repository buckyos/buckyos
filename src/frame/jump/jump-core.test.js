import { assertEquals } from 'jsr:@std/assert@1'
import {
  actionForPath,
  candidateFromFragment,
  normalizeZoneHostname,
  plan,
  readPreference,
  STORAGE_KEY,
  targetUrl,
  writePreference,
} from './jump-core.js'

class MemoryStorage {
  map = new Map()
  getItem(k) { return this.map.get(k) ?? null }
  setItem(k, v) { this.map.set(k, String(v)) }
  removeItem(k) { this.map.delete(k) }
}

Deno.test('hostname normalization rejects anything but a bare hostname', () => {
  assertEquals(normalizeZoneHostname(' Alice.Example.com. '), 'alice.example.com')
  for (const bad of ['https://a.example', 'u@a.example', 'a.example:443', 'a.example/x', 'a.example?q', 'a.example#f', 'a%2e.example', 'localhost', '']) {
    assertEquals(normalizeZoneHostname(bad), null, bad)
  }
})

Deno.test('only registered action paths resolve', () => {
  assertEquals(actionForPath('/sysdlg/app_installer'), 'app_installer')
  assertEquals(actionForPath('/sysdlg/app_installer/'), 'app_installer')
  assertEquals(actionForPath('/sysdlg/app_installer/index.html'), 'app_installer')
  assertEquals(actionForPath('/dlgs/evil'), null)
  assertEquals(targetUrl('a.example', 'app_installer', '#identifier=x&zone=evil.example'), 'https://a.example/sysdlg/app_installer?identifier=x')
  assertEquals(targetUrl('a.example', 'app_installer', ''), 'https://a.example/sysdlg/app_installer')
  assertEquals(targetUrl('https://a.example', 'app_installer', ''), null)
})

Deno.test('a third-party candidate never overrides the preference or auto-jumps', () => {
  const storage = new MemoryStorage()
  let decision = plan('/sysdlg/app_installer', '#zone=evil.example&app_url=x', storage)
  assertEquals(decision.kind, 'choose')
  assertEquals(decision.prefill, '')
  assertEquals(decision.candidate, 'evil.example')

  writePreference(storage, 'alice.example', false)
  decision = plan('/sysdlg/app_installer', '#zone=evil.example', storage)
  assertEquals(decision.kind, 'choose')
  assertEquals(decision.prefill, 'alice.example')

  writePreference(storage, 'alice.example', true)
  decision = plan('/sysdlg/app_installer', '#zone=evil.example&identifier=x', storage)
  assertEquals(decision, { kind: 'jump', url: 'https://alice.example/sysdlg/app_installer?identifier=x' })
})

Deno.test('broken or missing storage falls back to manual choice', () => {
  const storage = new MemoryStorage()
  storage.setItem(STORAGE_KEY, '{not json')
  assertEquals(readPreference(storage), null)
  assertEquals(readPreference(null), null)
  assertEquals(plan('/sysdlg/app_installer', '', null).kind, 'choose')
  assertEquals(candidateFromFragment('#zone=a.example:8080'), null)
})
