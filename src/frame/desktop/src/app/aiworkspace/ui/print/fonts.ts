/* The print fonts of a Workspace (打印 TODO §3.5, T6, Q17): root `config.fonts` maps `sans`, `serif` and `mono` to one
 * family each, from the default fonts (ext-core) and the font package (`ext-fonts`, one online item per family). The
 * print dialog checks these three: a family is there when it is a default font, or when its online item is ready on
 * every node that reports the package. That report (control-panel `ext_tools.status`) is an administrator's: when it
 * cannot be read the check says so instead of guessing. */

import { useCallback } from 'react'
import { ALL_FONTS, FONT_PACKAGE, GENERICS, fontChoice, missingFonts, readFontSettings, type FontSettings, type Generic, type MissingFont } from '@print/fonts'
import { fetchExtToolsStatus, type ExtToolsStatusResponse } from '../../../../api/settings'
import { useLoad } from '../../state/hooks'

/** The online items of the font package that are ready wherever the package is. */
export function readyFontItems(status: ExtToolsStatusResponse): Set<string> {
  const pkg = status.packages.find((p) => p.id === FONT_PACKAGE)
  const nodes = Object.values(pkg?.nodes ?? {})
  const items = new Set(nodes.flatMap((node) => Object.keys(node.online ?? {})))
  return new Set([...items].filter((item) => nodes.every((node) => node.online?.[item]?.state === 'ready')))
}

/** What a mapping needs from the nodes: the font package's items it names (none: nothing to ask). */
function packItems(settings: FontSettings): string[] {
  return GENERICS.flatMap((g) => { const item = settings[g] ? fontChoice(settings[g])?.item : null; return item ? [item] : [] })
}

export type FontCheck =
  | { state: 'checking' }
  /** `missing` is empty when every mapped font is there. */
  | { state: 'checked'; missing: MissingFont[] }
  /** The package's state could not be read: only the families nothing provides are known to be missing. */
  | { state: 'unknown'; missing: MissingFont[]; unchecked: string[] }

/** The check of the dialog (§3.5 检查), once when it opens and whenever the mapping changes. */
export function useFontCheck(value: unknown): FontCheck {
  const settings = readFontSettings(value)
  const key = JSON.stringify(settings)
  const load = useCallback(async (): Promise<FontCheck> => {
    const mapped = JSON.parse(key) as FontSettings
    const items = packItems(mapped)
    if (items.length === 0) return { state: 'checked', missing: missingFonts(mapped, new Set()) }
    const { data } = await fetchExtToolsStatus()
    if (data) return { state: 'checked', missing: missingFonts(mapped, readyFontItems(data)) }
    const unknown = missingFonts(mapped, new Set(items))
    return { state: 'unknown', missing: unknown, unchecked: GENERICS.flatMap((g) => (mapped[g] && fontChoice(mapped[g])?.item ? [mapped[g]] : [])) }
  }, [key])
  return useLoad(load, key).data ?? { state: 'checking' }
}

/** A mapping as it is written: only catalogue families of the right kind; null when nothing is mapped (the key goes). */
export function fontSettingsValue(choice: Partial<Record<Generic, string>>): Record<string, string> | null {
  const out: Record<string, string> = {}
  for (const g of GENERICS) {
    const family = choice[g]
    if (!family) continue
    if (!ALL_FONTS.some((f) => f.family === family && f.generic === g)) throw new Error(`${family} is not a ${g} font of the print`)
    out[g] = family
  }
  return Object.keys(out).length > 0 ? out : null
}
