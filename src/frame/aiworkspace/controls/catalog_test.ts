import { assert, assertEquals } from './test_assert.ts'
import { catalogJson, CONTROLS } from './catalog.ts'
import { CONTROL_KINDS } from './contract.ts'

Deno.test('aiws/controls.json is generated from the catalogue (deno run -A tools/build_catalog.ts)', async () => {
  const file = await Deno.readTextFile(new URL('../aiws/controls.json', import.meta.url))
  assertEquals(file, catalogJson())
})

Deno.test('every control kind is described for Agents', () => {
  assertEquals(CONTROLS.map((c) => c.kind), [...CONTROL_KINDS])
  for (const c of CONTROLS) {
    assert(c.summary && c.when && c.agent.includes('doc.commit'), c.kind)
    assert(c.agent.includes('"auto"'), `${c.kind}: the manual tells Agents not to compute coordinates`)
  }
})
