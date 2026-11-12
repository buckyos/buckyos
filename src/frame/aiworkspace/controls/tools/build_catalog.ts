/* Writes src/frame/aiworkspace/aiws/controls.json from the control catalogue (catalog.ts) and the public contracts of
 * the official data controls into aiws/contracts/<kind>.json (contracts.ts, 控件机制 TODO G13). */
import { catalogJson } from '../catalog.ts'
import { CONTRACTS, contractJson } from '../contracts.ts'

const out = new URL('../../aiws/controls.json', import.meta.url)
await Deno.writeTextFile(out, catalogJson())
console.log(`wrote ${out.pathname}`)
await Deno.mkdir(new URL('../../aiws/contracts/', import.meta.url), { recursive: true })
for (const kind of Object.keys(CONTRACTS)) {
  const file = new URL(`../../aiws/contracts/${kind}.json`, import.meta.url)
  await Deno.writeTextFile(file, contractJson(kind))
  console.log(`wrote ${file.pathname}`)
}
