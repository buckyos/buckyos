/* Sizes as a person reads them: `12 KiB`, `3.4 MiB` (binary units, as the asset limits are stated). */

export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0 B'
  const units = ['B', 'KiB', 'MiB', 'GiB']
  let i = 0
  let v = n
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i += 1 }
  const shown = i === 0 || v >= 100 || Number.isInteger(v) ? String(Math.round(v)) : v.toFixed(1)
  return `${shown} ${units[i]}`
}
