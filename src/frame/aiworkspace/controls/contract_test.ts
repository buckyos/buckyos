import { assertEquals } from './test_assert.ts'
import { CONTROL_KINDS, DATA_CONTROLS, TEXT_VIEWS } from './contract.ts'

Deno.test('the constants agree with core.json (the core reads that file)', async () => {
  const core = JSON.parse(await Deno.readTextFile(new URL('./core.json', import.meta.url)))
  assertEquals([...CONTROL_KINDS], core.control_kinds)
  assertEquals([...TEXT_VIEWS], core.text_views)
  assertEquals([...DATA_CONTROLS], core.data_controls)
})
