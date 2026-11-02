/* OOXML measures (Office 导入 TODO §2.1): DrawingML lengths in EMU (914 400 per inch, 12 700 per point), Word lengths
 * in twentieths of a point, Word font sizes in half points, DrawingML font sizes in hundredths of a point, angles in
 * 60 000ths of a degree, percentages in 1 000ths of a percent (or `50%` in the Strict schema). */

export const EMU_PER_PT = 12700

export const emuToPt = (emu: number) => emu / EMU_PER_PT
export const twipToPt = (twip: number) => twip / 20

/** A whole number attribute, or `fallback` when absent or broken. */
export function int(value: string | undefined, fallback = 0): number {
  if (value === undefined) return fallback
  const n = parseInt(value, 10)
  return Number.isFinite(n) ? n : fallback
}

/** An ST_Percentage / ST_PositiveFixedPercentage as a fraction (`50000` or `50%` → 0.5). */
export function fraction(value: string | undefined, fallback = 0): number {
  if (value === undefined) return fallback
  const v = value.trim()
  if (v.endsWith('%')) { const n = parseFloat(v); return Number.isFinite(n) ? n / 100 : fallback }
  const n = parseInt(v, 10)
  return Number.isFinite(n) ? n / 100000 : fallback
}

/** An ST_Angle in degrees. */
export const angle = (value: string | undefined) => int(value) / 60000

/** An on/off attribute (`w:b`, `w:val="0"`, `b="1"`): present without a value means on. */
export function onOff(value: string | undefined, present: boolean): boolean {
  if (!present) return false
  if (value === undefined) return true
  return !(value === '0' || value === 'false' || value === 'off' || value === 'none')
}
