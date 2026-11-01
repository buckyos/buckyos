/* DrawingML colours and themes (Office 导入 TODO §4.2): `a:theme` gives 12 scheme colours and the major / minor
 * fonts; a slide master's `p:clrMap` maps the background / text names (bg1, tx1 …) onto them. A colour is one of
 * srgbClr / scrgbClr / hslClr / sysClr / prstClr / schemeClr followed by modifiers applied in document order:
 * lumMod / lumOff / satMod / satOff / hueMod / hueOff in HSL (Office's, L in 0…1), tint / shade in linear RGB (as
 * LibreOffice does, gamma 2.3), alpha / alphaMod / alphaOff, comp / inv / gray. */

import type { XmlElement } from '../import-drawio/xml.ts'
import { attr, kid, kids, local } from './package.ts'
import { fraction, int } from './units.ts'

export interface FontSet { latin: string; ea: string; cs: string }
export interface Theme {
  /** dk1 lt1 dk2 lt2 accent1–6 hlink folHlink → `#rrggbb`. */
  colors: Map<string, string>
  major: FontSet
  minor: FontSet
}

/** Office 2013–2022's default theme, for parts without one. */
export const DEFAULT_THEME: Theme = {
  colors: new Map(Object.entries({ dk1: '#000000', lt1: '#ffffff', dk2: '#44546a', lt2: '#e7e6e6', accent1: '#4472c4', accent2: '#ed7d31', accent3: '#a5a5a5', accent4: '#ffc000', accent5: '#5b9bd5', accent6: '#70ad47', hlink: '#0563c1', folHlink: '#954f72' })),
  major: { latin: 'Calibri Light', ea: '', cs: '' },
  minor: { latin: 'Calibri', ea: '', cs: '' },
}

/** A master's default colour map. */
export const DEFAULT_CLR_MAP: Record<string, string> = { bg1: 'lt1', tx1: 'dk1', bg2: 'lt2', tx2: 'dk2', accent1: 'accent1', accent2: 'accent2', accent3: 'accent3', accent4: 'accent4', accent5: 'accent5', accent6: 'accent6', hlink: 'hlink', folHlink: 'folHlink' }

const COLOR_NAMES = new Set(['srgbClr', 'scrgbClr', 'hslClr', 'sysClr', 'prstClr', 'schemeClr'])

function fontSet(el: XmlElement | undefined, fallback: FontSet): FontSet {
  if (!el) return fallback
  const face = (name: string) => attr(kid(el, name), 'typeface') ?? ''
  // the script fonts (`<a:font script="Hans" …>`) stand in for an empty East Asian face
  const hans = kids(el, 'font').find((f) => f.attrs.script === 'Hans' || f.attrs.script === 'Hant' || f.attrs.script === 'Jpan')
  return { latin: face('latin') || fallback.latin, ea: face('ea') || (hans?.attrs.typeface ?? '') || fallback.ea, cs: face('cs') || fallback.cs }
}

export function readTheme(doc: XmlElement | null | undefined): Theme {
  if (!doc) return DEFAULT_THEME
  const elementsOf = kid(doc, 'themeElements')
  const scheme = kid(elementsOf, 'clrScheme')
  const colors = new Map(DEFAULT_THEME.colors)
  for (const c of scheme ? scheme.children : []) {
    if (typeof c === 'string') continue
    const value = c.children.find((x): x is XmlElement => typeof x !== 'string' && COLOR_NAMES.has(local(x.name)))
    const hex = value ? baseColor(value, { theme: DEFAULT_THEME, map: DEFAULT_CLR_MAP }) : null
    if (hex) colors.set(local(c.name), hex)
  }
  const fonts = kid(elementsOf, 'fontScheme')
  return { colors, major: fontSet(kid(fonts, 'majorFont'), DEFAULT_THEME.major), minor: fontSet(kid(fonts, 'minorFont'), DEFAULT_THEME.minor) }
}

/** A master's `p:clrMap` (or a layout's / slide's `p:clrMapOvr` override). */
export function readClrMap(el: XmlElement | undefined, base: Record<string, string> = DEFAULT_CLR_MAP): Record<string, string> {
  if (!el) return base
  const out = { ...base }
  for (const [k, v] of Object.entries(el.attrs)) if (!k.startsWith('xmlns')) out[local(k)] = v
  return out
}

export interface ColorContext {
  theme: Theme
  map: Record<string, string>
  /** The colour `phClr` stands for (a style matrix entry applied to a shape's style colour). */
  phClr?: Rgba | null
}
export interface Rgba { hex: string; alpha: number }

const PRESET: Record<string, string> = {
  black: '#000000', white: '#ffffff', red: '#ff0000', green: '#008000', blue: '#0000ff', yellow: '#ffff00', cyan: '#00ffff', magenta: '#ff00ff',
  gray: '#808080', grey: '#808080', darkGray: '#a9a9a9', lightGray: '#d3d3d3', orange: '#ffa500', purple: '#800080', navy: '#000080', maroon: '#800000',
  olive: '#808000', teal: '#008080', silver: '#c0c0c0', lime: '#00ff00', aqua: '#00ffff', fuchsia: '#ff00ff', pink: '#ffc0cb', brown: '#a52a2a', gold: '#ffd700',
  darkBlue: '#00008b', darkRed: '#8b0000', darkGreen: '#006400', ltGray: '#d3d3d3', dkGray: '#a9a9a9', dkBlue: '#00008b', dkRed: '#8b0000', dkGreen: '#006400',
}
const SYSTEM: Record<string, string> = { windowText: '#000000', window: '#ffffff', btnFace: '#f0f0f0', btnText: '#000000', highlight: '#0078d7', highlightText: '#ffffff', grayText: '#6d6d6d', menuText: '#000000', captionText: '#000000' }

const clamp01 = (v: number) => Math.min(1, Math.max(0, v))
const hex2 = (v: number) => Math.round(clamp01(v) * 255).toString(16).padStart(2, '0')
const toHex = (r: number, g: number, b: number) => `#${hex2(r)}${hex2(g)}${hex2(b)}`
function fromHex(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16)
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255]
}

export function rgbToHsl(r: number, g: number, b: number): [number, number, number] {
  const max = Math.max(r, g, b), min = Math.min(r, g, b)
  const l = (max + min) / 2
  if (max === min) return [0, 0, l]
  const d = max - min
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min)
  const h = max === r ? (g - b) / d + (g < b ? 6 : 0) : max === g ? (b - r) / d + 2 : (r - g) / d + 4
  return [h * 60, s, l]
}

export function hslToRgb(h: number, s: number, l: number): [number, number, number] {
  if (s === 0) return [l, l, l]
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s
  const p = 2 * l - q
  const hue = (t: number) => {
    let x = t
    if (x < 0) x += 1
    if (x > 1) x -= 1
    if (x < 1 / 6) return p + (q - p) * 6 * x
    if (x < 1 / 2) return q
    if (x < 2 / 3) return p + (q - p) * (2 / 3 - x) * 6
    return p
  }
  const k = (((h % 360) + 360) % 360) / 360
  return [hue(k + 1 / 3), hue(k), hue(k - 1 / 3)]
}

const GAMMA = 2.3
const toLinear = (v: number) => Math.pow(clamp01(v), GAMMA)
const fromLinear = (v: number) => Math.pow(clamp01(v), 1 / GAMMA)

/** The colour an element names before its modifiers (`#rrggbb`), or null. */
function baseColor(el: XmlElement, ctx: ColorContext): string | null {
  switch (local(el.name)) {
    case 'srgbClr': { const v = (attr(el, 'val') ?? '').toLowerCase(); return /^[0-9a-f]{6}$/.test(v) ? `#${v}` : null }
    case 'scrgbClr': return toHex(fromLinear(fraction(attr(el, 'r'))), fromLinear(fraction(attr(el, 'g'))), fromLinear(fraction(attr(el, 'b'))))
    case 'hslClr': { const [r, g, b] = hslToRgb(int(attr(el, 'hue')) / 60000, fraction(attr(el, 'sat')), fraction(attr(el, 'lum'))); return toHex(r, g, b) }
    case 'sysClr': { const last = (attr(el, 'lastClr') ?? '').toLowerCase(); return /^[0-9a-f]{6}$/.test(last) ? `#${last}` : SYSTEM[attr(el, 'val') ?? ''] ?? null }
    case 'prstClr': return PRESET[attr(el, 'val') ?? ''] ?? null
    case 'schemeClr': {
      const name = attr(el, 'val') ?? ''
      if (name === 'phClr') return ctx.phClr?.hex ?? null
      const mapped = ctx.map[name] ?? name
      return ctx.theme.colors.get(mapped) ?? ctx.theme.colors.get(name) ?? null
    }
  }
  return null
}

/** A colour element with its modifiers applied. */
export function colorOf(el: XmlElement, ctx: ColorContext): Rgba | null {
  const base = baseColor(el, ctx)
  if (!base) return null
  let [r, g, b] = fromHex(base)
  let alpha = local(el.name) === 'schemeClr' && attr(el, 'val') === 'phClr' ? ctx.phClr?.alpha ?? 1 : 1
  for (const m of el.children) {
    if (typeof m === 'string') continue
    const v = fraction(attr(m, 'val'), 1)
    const name = local(m.name)
    if (name === 'alpha') { alpha = clamp01(v); continue }
    if (name === 'alphaMod') { alpha = clamp01(alpha * v); continue }
    if (name === 'alphaOff') { alpha = clamp01(alpha + v); continue }
    if (name === 'tint' || name === 'shade') {
      const k = clamp01(v)
      const f = name === 'tint' ? (c: number) => fromLinear(1 - (1 - toLinear(c)) * k) : (c: number) => fromLinear(toLinear(c) * k)
      r = f(r); g = f(g); b = f(b)
      continue
    }
    if (name === 'comp') { const [h, s, l] = rgbToHsl(r, g, b); [r, g, b] = hslToRgb(h + 180, s, l); continue }
    if (name === 'inv') { r = 1 - r; g = 1 - g; b = 1 - b; continue }
    if (name === 'gray') { const y = 0.299 * r + 0.587 * g + 0.114 * b; r = g = b = y; continue }
    if (name === 'red' || name === 'green' || name === 'blue') { if (name === 'red') r = v; else if (name === 'green') g = v; else b = v; continue }
    const hsl = rgbToHsl(r, g, b)
    let [h, s, l] = hsl
    switch (name) {
      case 'lumMod': l *= v; break
      case 'lumOff': l += v; break
      case 'lum': l = v; break
      case 'satMod': s *= v; break
      case 'satOff': s += v; break
      case 'sat': s = v; break
      case 'hueMod': h *= v; break
      case 'hueOff': h += int(attr(m, 'val')) / 60000; break
      case 'hue': h = int(attr(m, 'val')) / 60000; break
      default: continue
    }
    ;[r, g, b] = hslToRgb(h, clamp01(s), clamp01(l))
  }
  return { hex: toHex(r, g, b), alpha }
}

/** The first colour element inside `parent` (a solidFill, a gradient stop, a style reference …). */
export function colorIn(parent: XmlElement | undefined, ctx: ColorContext): Rgba | null {
  if (!parent) return null
  const el = parent.children.find((c): c is XmlElement => typeof c !== 'string' && COLOR_NAMES.has(local(c.name)))
  return el ? colorOf(el, ctx) : null
}

/** A theme font reference (`+mj-lt`, `+mn-ea` …) resolved to a face name. */
export function themeFont(face: string, theme: Theme): string {
  const m = /^\+(mj|mn)-(lt|ea|cs)$/.exec(face)
  if (!m) return face
  const set = m[1] === 'mj' ? theme.major : theme.minor
  return m[2] === 'lt' ? set.latin : m[2] === 'ea' ? set.ea : set.cs
}
