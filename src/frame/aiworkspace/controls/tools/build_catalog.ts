/* Writes src/frame/aiworkspace/aiws/controls.json from the control catalogue (catalog.ts). */
import { catalogJson } from '../catalog.ts'

const out = new URL('../../aiws/controls.json', import.meta.url)
await Deno.writeTextFile(out, catalogJson())
console.log(`wrote ${out.pathname}`)
