/* The font catalogue (打印 TODO T6, T11). Run: `deno test -A` in this folder. The online items are checked against the
 * font package's manifest by publish/exttool/ext-fonts/test_ext_fonts.py. */

import { assert, assertEquals } from './test_assert.ts'
import { ALL_FONTS, DEFAULT_FONTS, GENERICS, PACK_FONTS, SYSTEM_DEFAULT, fontChoice, missingFonts } from './fonts.ts'

Deno.test('fonts: one entry per family, every pack font in its own online item', () => {
  const families = ALL_FONTS.map((f) => f.family)
  assertEquals(new Set(families).size, families.length)
  const items = PACK_FONTS.map((f) => f.item)
  assert(items.every((item) => item !== null && /^[a-z0-9-]+$/.test(item)))
  assertEquals(new Set(items).size, items.length)
  assert(DEFAULT_FONTS.every((f) => f.item === null))
  for (const g of GENERICS) assertEquals(fontChoice(SYSTEM_DEFAULT[g])?.item, null, `the default ${g} is a default font`)
})

Deno.test('fonts: a default font is never missing, a pack font is until its item is ready', () => {
  assertEquals(missingFonts({ sans: 'Noto Sans Arabic', serif: 'Noto Serif Hebrew', mono: 'Noto Sans Mono CJK JP' }, new Set()), [])
  assertEquals(missingFonts({ serif: 'Noto Serif Devanagari', mono: 'JetBrains Mono' }, new Set(['jetbrains-mono'])), [
    { generic: 'serif', family: 'Noto Serif Devanagari', item: 'noto-serif-devanagari' },
  ])
})
