/* draw.io built-in shapes that its editor draws in script (Shapes.js), not from a stencil: the common ones as SVG
 * shape bodies (draw.io 导入 TODO §2.4), so an import keeps their outline and their colours follow the shape's
 * fill and stroke. Each is drawn in a 100 × 100 box and stretched. */

import type { SvgBody } from './shapes.ts'

const FS = 'fill:var(--aiws-shape-fill);stroke:var(--aiws-shape-stroke);stroke-width:var(--aiws-shape-stroke-width);vector-effect:non-scaling-stroke;stroke-linejoin:round'
const S = 'fill:none;stroke:var(--aiws-shape-stroke);stroke-width:var(--aiws-shape-stroke-width);vector-effect:non-scaling-stroke;stroke-linecap:round;stroke-linejoin:round'

const path = (d: string, style = FS) => `<path d="${d}" style="${style}"/>`
const ellipse = (cx: number, cy: number, rx: number, ry: number, style = FS) => `<ellipse cx="${cx}" cy="${cy}" rx="${rx}" ry="${ry}" style="${style}"/>`

/** `title`: an English name, for developers (it is not shown). */
const BODIES: Record<string, { title: string; body: string; aspect?: 'fixed' }> = {
  actor: { title: 'Actor', body: path('M0 100C0 66 10 46 50 46C90 46 100 66 100 100Z') + ellipse(50, 22, 21, 21) },
  umlActor: { title: 'UML actor', body: ellipse(50, 12.5, 25, 12.5) + path('M50 25L50 66.7M0 33.3L100 33.3M50 66.7L0 100M50 66.7L100 100', S) },
  tape: { title: 'Tape', body: path('M0 10Q25 30 50 10T100 10L100 90Q75 70 50 90T0 90Z') },
  internalStorage: { title: 'Internal storage', body: path('M0 0L100 0L100 100L0 100Z') + path('M20 0L20 100M0 20L100 20', S) },
  manualInput: { title: 'Manual input', body: path('M0 30L100 0L100 100L0 100Z') },
  dataStorage: { title: 'Data storage', body: path('M15 0L100 0Q80 50 100 100L15 100Q-5 50 15 0Z') },
  delay: { title: 'Delay', body: path('M0 0L50 0C116 0 116 100 50 100L0 100Z') },
  display: { title: 'Display', body: path('M0 50L20 0L80 0C106 0 106 100 80 100L20 100Z') },
  offPageConnector: { title: 'Off-page connector', body: path('M0 0L100 0L100 75L50 100L0 75Z') },
  sumEllipse: { title: 'Summing junction', body: ellipse(50, 50, 50, 50) + path('M14.6 14.6L85.4 85.4M85.4 14.6L14.6 85.4', S) },
  orEllipse: { title: 'Or', body: ellipse(50, 50, 50, 50) + path('M0 50L100 50M50 0L50 100', S) },
  cube: { title: 'Cube', body: path('M0 20L80 20L80 100L0 100Z') + path('M0 20L20 0L100 0L80 20Z') + path('M80 20L100 0L100 80L80 100Z') },
  folder: { title: 'Folder', body: path('M0 15L0 100L100 100L100 15L46 15L40 0L0 0Z') },
  component: { title: 'UML component', body: path('M10 0L100 0L100 100L10 100Z') + path('M0 18L20 18L20 36L0 36Z') + path('M0 64L20 64L20 82L0 82Z') },
  umlBoundary: { title: 'UML boundary', body: ellipse(60, 50, 40, 50) + path('M0 20L0 80M0 50L20 50', S), aspect: 'fixed' },
  umlEntity: { title: 'UML entity', body: ellipse(50, 45, 45, 45) + path('M5 100L95 100', S), aspect: 'fixed' },
  umlControl: { title: 'UML control', body: ellipse(50, 55, 45, 45) + path('M50 10L62 2M50 10L62 18', S), aspect: 'fixed' },
  lollipop: { title: 'Interface (lollipop)', body: ellipse(80, 50, 20, 20) + path('M0 50L60 50', S) },
  waypoint: { title: 'Waypoint', body: ellipse(50, 50, 50, 50) },
  corner: { title: 'Corner', body: path('M0 0L100 0L100 25L25 25L25 100L0 100Z') },
  tee: { title: 'Tee', body: path('M0 0L100 0L100 25L62.5 25L62.5 100L37.5 100L37.5 25L0 25Z') },
  plus: { title: 'Plus', body: path('M37.5 0L62.5 0L62.5 37.5L100 37.5L100 62.5L62.5 62.5L62.5 100L37.5 100L37.5 62.5L0 62.5L0 37.5L37.5 37.5Z') },
}

export function builtinSvg(name: string): (SvgBody & { title: string }) | null {
  const entry = BODIES[name]
  return entry ? { viewBox: '0 0 100 100', body: entry.body, aspect: entry.aspect ?? 'stretch', title: entry.title } : null
}

export const BUILTIN_SVG_NAMES = Object.keys(BODIES)
