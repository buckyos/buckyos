/* The fonts a print may use (打印 TODO §3.5, Q16, Q17, T6, T11): the default fonts every node has (ext-core) and the
 * fonts of the font package (`ext-fonts`, one online item per font family, downloaded by an administrator). A
 * Workspace maps the three generic families `sans`, `serif` and `mono` to one family each (root `config.fonts`); what
 * it does not map is the default. The print checks exactly these three. Family names are those fontconfig reports. */

export type Generic = 'sans' | 'serif' | 'mono'
export const GENERICS: Generic[] = ['sans', 'serif', 'mono']

/** The font package whose online items hold the downloadable fonts. */
export const FONT_PACKAGE = 'ext-fonts'

export interface FontChoice {
  family: string
  generic: Generic
  /** `null`: a default font (ext-core). Otherwise the online item of `ext-fonts` that holds it. */
  item: string | null
  /** What the font is for, as BCP 47 language or script tags. */
  langs: string[]
}

/** ext-core: the fonts its fontconfig gives Chromium on a Linux node (LibreOffice's font directory, with Noto CJK added),
 * as fontconfig names them. A selection: the faces of LibreOffice's own documents (Amiri, Rubik, the CLM faces…) are
 * there too and left out here. */
const LATIN = ['Latn', 'Cyrl', 'Grek']
const CJK: [string, string[]][] = [['SC', ['zh-Hans']], ['TC', ['zh-Hant']], ['HK', ['zh-HK']], ['JP', ['ja']], ['KR', ['ko']]]
export const DEFAULT_FONTS: FontChoice[] = [
  ...CJK.map(([r, langs]): FontChoice => ({ family: `Noto Sans CJK ${r}`, generic: 'sans', item: null, langs })),
  ...CJK.map(([r, langs]): FontChoice => ({ family: `Noto Serif CJK ${r}`, generic: 'serif', item: null, langs })),
  ...CJK.map(([r, langs]): FontChoice => ({ family: `Noto Sans Mono CJK ${r}`, generic: 'mono', item: null, langs })),
  { family: 'Noto Sans', generic: 'sans', item: null, langs: LATIN },
  { family: 'Noto Serif', generic: 'serif', item: null, langs: LATIN },
  { family: 'DejaVu Sans', generic: 'sans', item: null, langs: LATIN },
  { family: 'DejaVu Serif', generic: 'serif', item: null, langs: LATIN },
  { family: 'DejaVu Sans Mono', generic: 'mono', item: null, langs: LATIN },
  { family: 'Liberation Sans', generic: 'sans', item: null, langs: LATIN },
  { family: 'Liberation Serif', generic: 'serif', item: null, langs: LATIN },
  { family: 'Liberation Mono', generic: 'mono', item: null, langs: LATIN },
  { family: 'Carlito', generic: 'sans', item: null, langs: LATIN },
  { family: 'Caladea', generic: 'serif', item: null, langs: LATIN },
  { family: 'Noto Sans Arabic', generic: 'sans', item: null, langs: ['ar', 'fa', 'ur', 'Arab'] },
  { family: 'Noto Naskh Arabic', generic: 'serif', item: null, langs: ['ar', 'fa', 'ur', 'Arab'] },
  { family: 'Noto Sans Hebrew', generic: 'sans', item: null, langs: ['he', 'Hebr'] },
  { family: 'Noto Serif Hebrew', generic: 'serif', item: null, langs: ['he', 'Hebr'] },
  { family: 'Noto Sans Armenian', generic: 'sans', item: null, langs: ['hy', 'Armn'] },
  { family: 'Noto Serif Armenian', generic: 'serif', item: null, langs: ['hy', 'Armn'] },
  { family: 'Noto Sans Georgian', generic: 'sans', item: null, langs: ['ka', 'Geor'] },
  { family: 'Noto Serif Georgian', generic: 'serif', item: null, langs: ['ka', 'Geor'] },
  { family: 'Noto Sans Lao', generic: 'sans', item: null, langs: ['lo', 'Laoo'] },
  { family: 'Noto Serif Lao', generic: 'serif', item: null, langs: ['lo', 'Laoo'] },
]

/** `ext-fonts`: one online item per family (publish/exttool/ext-fonts/ext-manifest.json is the source of truth; its
 * test checks this list against it). Arabic and Hebrew are default fonts already. */
export const PACK_FONTS: FontChoice[] = [
  { family: 'Noto Sans Thai', generic: 'sans', item: 'noto-sans-thai', langs: ['th', 'Thai'] },
  { family: 'Noto Serif Thai', generic: 'serif', item: 'noto-serif-thai', langs: ['th', 'Thai'] },
  { family: 'Noto Sans Devanagari', generic: 'sans', item: 'noto-sans-devanagari', langs: ['hi', 'mr', 'ne', 'Deva'] },
  { family: 'Noto Serif Devanagari', generic: 'serif', item: 'noto-serif-devanagari', langs: ['hi', 'mr', 'ne', 'Deva'] },
  { family: 'Inter', generic: 'sans', item: 'inter', langs: LATIN },
  { family: 'Source Sans 3', generic: 'sans', item: 'source-sans-3', langs: LATIN },
  { family: 'Source Serif 4', generic: 'serif', item: 'source-serif-4', langs: LATIN },
  { family: 'JetBrains Mono', generic: 'mono', item: 'jetbrains-mono', langs: LATIN },
]

export const ALL_FONTS: FontChoice[] = [...DEFAULT_FONTS, ...PACK_FONTS]

/** What a node uses when the Workspace maps nothing: the ext-core rules (Simplified Chinese first, §3.5). */
export const SYSTEM_DEFAULT: Record<Generic, string> = { sans: 'Noto Sans CJK SC', serif: 'Noto Serif CJK SC', mono: 'DejaVu Sans Mono' }

/** Root `config.fonts`. */
export type FontSettings = Partial<Record<Generic, string>>

export function readFontSettings(value: unknown): FontSettings {
  const out: FontSettings = {}
  if (!value || typeof value !== 'object' || Array.isArray(value)) return out
  for (const g of GENERICS) {
    const v = (value as Record<string, unknown>)[g]
    if (typeof v === 'string' && v.trim() && v.length <= 128) out[g] = v.trim()
  }
  return out
}

export function fontChoice(family: string): FontChoice | null {
  return ALL_FONTS.find((f) => f.family === family) ?? null
}

/** A missing font of the three mappings: one whose online item is not ready on the node (§3.5 检查). A family the
 * catalogue does not know is missing too (nothing provides it). */
export interface MissingFont {
  generic: Generic
  family: string
  item: string | null
}

/** `ready`: the online items of `ext-fonts` that are `ready` on the node that renders. */
export function missingFonts(settings: FontSettings, ready: ReadonlySet<string>): MissingFont[] {
  const out: MissingFont[] = []
  for (const g of GENERICS) {
    const family = settings[g]
    if (!family) continue
    const choice = fontChoice(family)
    if (!choice) out.push({ generic: g, family, item: null })
    else if (choice.item && !ready.has(choice.item)) out.push({ generic: g, family, item: choice.item })
  }
  return out
}

/** The CSS stack of a generic family in a print: the mapped family, then the node's defaults. */
export function cssStack(settings: FontSettings, g: Generic): string {
  const css = { sans: 'sans-serif', serif: 'serif', mono: 'monospace' }[g]
  const names = [settings[g], SYSTEM_DEFAULT[g]].filter((v, i, a): v is string => Boolean(v) && a.indexOf(v) === i)
  return [...names.map((n) => `"${n.replace(/["\\]/g, '')}"`), css].join(', ')
}
