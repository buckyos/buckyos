/* draw.io (mxGraph) cell styles: `name;key=value;…`. A bare token names a style of the default stylesheet
 * (draw.io's `default.xml`), whose keys apply first; explicit keys override them. Values keep their text; the
 * readers below turn them into numbers, colours and flags. */

export interface Style {
  /** The style as written (kept for export and re-import). */
  raw: string
  /** Named styles referenced by bare tokens, in order. */
  names: string[]
  /** Explicit keys as written (the last one wins). */
  keys: Map<string, string>
}

export function parseStyle(raw: string | undefined | null): Style {
  const names: string[] = []
  const keys = new Map<string, string>()
  for (const token of (raw ?? '').split(';')) {
    const t = token.trim()
    if (!t) continue
    const eq = t.indexOf('=')
    if (eq < 0) names.push(t)
    else keys.set(t.slice(0, eq).trim(), t.slice(eq + 1))
  }
  return { raw: raw ?? '', names, keys }
}

export function styleString(keys: Iterable<[string, string]>, names: string[] = []): string {
  return [...names, ...[...keys].map(([k, v]) => `${k}=${v}`)].join(';') + ';'
}

/** draw.io's default stylesheet (the named styles its palettes use as bare tokens). */
const NAMED: Record<string, Record<string, string>> = {
  defaultVertex: { shape: 'label', fontSize: '12', align: 'center', verticalAlign: 'middle', fillColor: 'default', strokeColor: 'default', fontColor: 'default' },
  defaultEdge: { shape: 'connector', endArrow: 'classic', fontSize: '11', align: 'center', verticalAlign: 'middle', rounded: '1', strokeColor: 'default', fontColor: 'default', labelBackgroundColor: 'default' },
  text: { fillColor: 'none', gradientColor: 'none', strokeColor: 'none', align: 'left', verticalAlign: 'top' },
  edgeLabel: { fillColor: 'none', gradientColor: 'none', strokeColor: 'none', align: 'left', verticalAlign: 'top', labelBackgroundColor: 'default', fontSize: '11' },
  label: { fontStyle: '1', align: 'left', verticalAlign: 'middle', spacing: '2', spacingLeft: '52', imageWidth: '42', imageHeight: '42', rounded: '1' },
  icon: { fontStyle: '0', align: 'center', verticalAlign: 'top', spacing: '0', spacingLeft: '0', spacingTop: '6', imageWidth: '48', imageHeight: '48', rounded: '1', imageAlign: 'center', verticalLabelPosition: 'bottom', labelBackgroundColor: 'default' },
  swimlane: { shape: 'swimlane', fontSize: '12', fontStyle: '1', startSize: '23' },
  group: { verticalAlign: 'top', fillColor: 'none', strokeColor: 'none', gradientColor: 'none', pointerEvents: '0' },
  ellipse: { shape: 'ellipse', perimeter: 'ellipsePerimeter' },
  doubleEllipse: { shape: 'doubleEllipse', perimeter: 'ellipsePerimeter' },
  rhombus: { shape: 'rhombus', perimeter: 'rhombusPerimeter' },
  triangle: { shape: 'triangle', perimeter: 'trianglePerimeter' },
  line: { shape: 'line', strokeWidth: '4', labelBackgroundColor: 'default', verticalAlign: 'top', spacingTop: '8' },
  image: { shape: 'image', labelBackgroundColor: 'default', verticalAlign: 'top', verticalLabelPosition: 'bottom' },
  roundImage: { shape: 'image', perimeter: 'ellipsePerimeter', labelBackgroundColor: 'default', verticalAlign: 'top', verticalLabelPosition: 'bottom' },
  rhombusImage: { shape: 'image', perimeter: 'rhombusPerimeter', labelBackgroundColor: 'default', verticalAlign: 'top', verticalLabelPosition: 'bottom' },
  arrow: { shape: 'arrow', edgeStyle: 'none', fillColor: 'default' },
}

/** The effective keys of a vertex or edge: the default style, the named styles, then the explicit keys. A bare
 * token that is no named style but a shape name (`hexagon;…`, `cloud;…`) sets the shape, as draw.io does. */
export function resolveStyle(style: Style, edge: boolean): Map<string, string> {
  const out = new Map<string, string>(Object.entries(NAMED[edge ? 'defaultEdge' : 'defaultVertex']))
  for (const name of style.names) {
    const named = NAMED[name]
    if (named) for (const [k, v] of Object.entries(named)) out.set(k, v)
    else if (!edge) out.set('shape', name)
  }
  for (const [k, v] of style.keys) out.set(k, v)
  return out
}

export function num(map: Map<string, string>, key: string, fallback: number): number {
  const v = map.get(key)
  if (v === undefined || v.trim() === '') return fallback
  const n = Number(v)
  return Number.isFinite(n) ? n : fallback
}

export function flag(map: Map<string, string>, key: string, fallback = false): boolean {
  const v = map.get(key)
  if (v === undefined) return fallback
  return v === '1' || v === 'true'
}

/** A colour value: `#rrggbb` (short forms and `rgb()` widened), `none`, `default` (theme colour), or null when
 * absent or not understood. */
export function color(map: Map<string, string>, key: string): string | 'none' | 'default' | null {
  const v = map.get(key)?.trim()
  if (!v) return null
  return normalizeColor(v)
}

const CSS_NAMES: Record<string, string> = {
  black: '#000000', white: '#ffffff', red: '#ff0000', green: '#008000', blue: '#0000ff', yellow: '#ffff00', orange: '#ffa500',
  gray: '#808080', grey: '#808080', lightgray: '#d3d3d3', lightgrey: '#d3d3d3', darkgray: '#a9a9a9', silver: '#c0c0c0', purple: '#800080',
  navy: '#000080', teal: '#008080', maroon: '#800000', olive: '#808000', lime: '#00ff00', aqua: '#00ffff', cyan: '#00ffff', fuchsia: '#ff00ff', magenta: '#ff00ff',
}

export function normalizeColor(value: string): string | 'none' | 'default' | null {
  const v = value.trim().toLowerCase()
  if (v === 'none' || v === 'transparent') return 'none'
  if (v === 'default' || v === 'inherit') return 'default'
  if (/^#[0-9a-f]{6}$/.test(v)) return v
  if (/^#[0-9a-f]{3}$/.test(v)) return `#${v[1]}${v[1]}${v[2]}${v[2]}${v[3]}${v[3]}`
  if (/^#[0-9a-f]{8}$/.test(v)) return v.slice(0, 7)
  const rgb = /^rgba?\(\s*(\d+)[\s,]+(\d+)[\s,]+(\d+)/.exec(v)
  if (rgb) return `#${[rgb[1], rgb[2], rgb[3]].map((c) => Math.min(255, Number(c)).toString(16).padStart(2, '0')).join('')}`
  return CSS_NAMES[v] ?? null
}
