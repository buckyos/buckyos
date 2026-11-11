/** Mock-only thumbnails: a tinted square with the name's initial, so image grids look like images. */

export function mockHue(name: string): number {
  let h = 0
  for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) % 360
  return h
}

export function mockThumbnailUrl(name: string): string {
  const initial = (name.trim()[0] ?? '?').toUpperCase().replace(/[<>&"']/g, '')
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" viewBox="0 0 96 96"><rect width="96" height="96" fill="hsl(${mockHue(name)} 55% 62%)"/><text x="48" y="60" font-family="sans-serif" font-size="40" fill="#fff" text-anchor="middle">${initial}</text></svg>`
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`
}
