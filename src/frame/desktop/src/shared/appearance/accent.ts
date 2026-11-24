/* ── The colour the user gave this Zone ──
 *
 * One per account and Zone. It only marks things as belonging to the Zone (its name in lists and menus, the edge of
 * an app window in the client) and is what apps get as `frame.init` `theme.accent`; the light and dark base colours
 * do not change with it.
 */

/** `#RRGGBB`, lower case. */
export type AccentColor = string

/** Colours offered in Settings; all of them carry white text and stand out on both base themes. */
export const accentPresets: readonly AccentColor[] = [
  '#406ad5',
  '#0f7f8c',
  '#2e7d4f',
  '#a8620c',
  '#c4453a',
  '#b8437f',
  '#7a55c7',
  '#5b6472',
]

/** A colour as it is stored, or null when it is none. */
export function normalizeAccent(value: unknown): AccentColor | null {
  return typeof value === 'string' && /^#[0-9a-f]{6}$/i.test(value) ? value.toLowerCase() : null
}

function luminance(color: AccentColor): number {
  const [r, g, b] = [1, 3, 5].map((index) => {
    const channel = parseInt(color.slice(index, index + 2), 16) / 255
    return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4
  })
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

/** The text colour that reads better on the accent. */
export function accentText(color: AccentColor): '#ffffff' | '#1b1b1a' {
  const onWhite = 1.05 / (luminance(color) + 0.05)
  const onDark = (luminance(color) + 0.05) / (luminance('#1b1b1a') + 0.05)
  return onWhite >= onDark ? '#ffffff' : '#1b1b1a'
}
