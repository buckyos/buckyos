import { assert, assertEquals } from './test_assert.ts'
import { catalogJson, CONTROLS } from './catalog.ts'
import { CONTROL_KINDS, DATA_CONTROLS, isDataControl } from './contract.ts'
import { CONTRACTS, contractJson } from './contracts.ts'

Deno.test('aiws/controls.json is generated from the catalogue (deno run -A tools/build_catalog.ts)', async () => {
  const file = await Deno.readTextFile(new URL('../aiws/controls.json', import.meta.url))
  assertEquals(file, catalogJson())
})

Deno.test('every control kind is described for Agents', () => {
  assertEquals(CONTROLS.map((c) => c.kind), [...CONTROL_KINDS])
  for (const c of CONTROLS) {
    assert(c.summary && c.when && c.agent.includes('doc.commit'), c.kind)
    // a layout control's manual tells Agents not to compute coordinates; a data control's that it is sized like a Block
    assert(c.agent.includes('"auto"'), `${c.kind}: the manual says how placement works`)
    if (isDataControl(c.kind)) assert(c.agent.includes('table.insert_records'), `${c.kind}: items are written as records`)
  }
})

Deno.test('the official data controls publish their contracts (aiws/contracts/<kind>.json, generated)', async () => {
  assertEquals(Object.keys(CONTRACTS).sort(), [...DATA_CONTROLS].sort())
  for (const kind of Object.keys(CONTRACTS)) {
    assertEquals(await Deno.readTextFile(new URL(`../aiws/contracts/${kind}.json`, import.meta.url)), contractJson(kind), kind)
    const c = CONTRACTS[kind]() as { ident: { name: string }; templates: Record<string, { nodes: { id: string; view: string; role?: string }[] }>; layouts: Record<string, { template?: string }>; gestures: { role: string }[] }
    assertEquals(c.ident.name, kind)
    // templates are label / shape / asset nodes (Q1) with unique ids; every layout names one of them; a gesture's role is a template's
    const roles = new Set<string>(['background'])
    for (const [name, t] of Object.entries(c.templates)) {
      assertEquals(new Set(t.nodes.map((n) => n.id)).size, t.nodes.length, name)
      for (const n of t.nodes) { assert(['label', 'shape', 'asset'].includes(n.view), `${name}.${n.id}`); roles.add(n.role ?? 'body') }
    }
    for (const [name, l] of Object.entries(c.layouts)) assert(!l.template || l.template in c.templates, `${kind} layout ${name}`)
    for (const g of c.gestures) assert(roles.has(g.role), `${kind} gesture ${g.role}`)
  }
})
