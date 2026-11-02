/* DrawingML geometry (Office 导入 TODO §4.3): a preset (`a:prstGeom`, the table in presets.ts) or custom
 * (`a:custGeom`) geometry is adjust values and guides — formulas over the shape's size — and paths over them. This
 * evaluates the guides for a w × h box and writes each path as SVG path data in that box, mirrored by flipH / flipV.
 *
 *   formulas   `*\/ +- +/ ?: abs at2 cat2 cos max min mod pin sat2 sin sqrt tan val` over numbers, the built-in
 *              names (w h l t r b hc vc ls ss wdN hdN ssdN cdN …) and earlier guides; angles in 60 000ths of a degree
 *   arcTo      the stated angles are visual (where the ray at that angle meets the ellipse); they are turned into the
 *              ellipse's own parameter (as Apache POI and LibreOffice do) to place the centre and the end point
 *   paths      their own coordinate space when `w` / `h` are given (custom geometry), scaled into the box; `fill`
 *              none / lighten / darken … and `stroke` false are kept for the SVG writer */

import type { XmlElement } from '../import-drawio/xml.ts'
import { attr, kid, kids, local } from './package.ts'
import { PRESETS } from './presets.ts'

export interface PresetPath { w?: number; h?: number; fill?: string; stroke?: boolean; cmds: (string | number)[][] }
export interface PresetDef { av: [string, string][]; gd: [string, string][]; paths: PresetPath[] }

/** One evaluated path: SVG path data in the box, how it is filled (`norm`, `none`, `lighten` …) and whether stroked. */
export interface GeomPath { d: string; fill: string; stroke: boolean }

const DEG = 60000
const rad = (a: number) => (a / DEG) * (Math.PI / 180)

function builtins(w: number, h: number): Map<string, number> {
  const ss = Math.min(w, h)
  const m = new Map<string, number>([
    ['w', w], ['h', h], ['l', 0], ['t', 0], ['r', w], ['b', h], ['hc', w / 2], ['vc', h / 2], ['ls', Math.max(w, h)], ['ss', ss],
    ['cd2', 10800000], ['cd4', 5400000], ['cd8', 2700000], ['3cd4', 16200000], ['3cd8', 8100000], ['5cd8', 13500000], ['7cd8', 18900000],
  ])
  for (const n of [2, 3, 4, 5, 6, 8, 10, 12, 16, 32]) { m.set(`wd${n}`, w / n); m.set(`hd${n}`, h / n); m.set(`ssd${n}`, ss / n) }
  return m
}

function evalFormula(fmla: string, vars: Map<string, number>): number {
  const [op, ...args] = fmla.trim().split(/\s+/)
  const v = args.map((a) => { const n = Number(a); return Number.isFinite(n) ? n : vars.get(a) ?? 0 })
  const [x = 0, y = 0, z = 0] = v
  switch (op) {
    case 'val': return x
    case '*/': return z === 0 ? 0 : (x * y) / z
    case '+-': return x + y - z
    case '+/': return z === 0 ? 0 : (x + y) / z
    case '?:': return x > 0 ? y : z
    case 'abs': return Math.abs(x)
    case 'at2': return (Math.atan2(y, x) * 180 / Math.PI) * DEG
    case 'cat2': return x * Math.cos(Math.atan2(z, y))
    case 'sat2': return x * Math.sin(Math.atan2(z, y))
    case 'cos': return x * Math.cos(rad(y))
    case 'sin': return x * Math.sin(rad(y))
    case 'tan': return x * Math.tan(rad(y))
    case 'max': return Math.max(x, y)
    case 'min': return Math.min(x, y)
    case 'mod': return Math.sqrt(x * x + y * y + z * z)
    case 'pin': return y < x ? x : y > z ? z : y
    case 'sqrt': return Math.sqrt(Math.max(0, x))
  }
  return 0
}

const n2 = (v: number) => String(Math.round(v * 100) / 100)

/** Evaluate a geometry in a w × h box. `adjust` overrides adjust values by name (`adj`, `adj1` …, formulas). */
export function evaluate(def: PresetDef, w: number, h: number, adjust: Record<string, string> = {}, flipH = false, flipV = false): GeomPath[] {
  const vars = builtins(w, h)
  for (const [name, fmla] of def.av) vars.set(name, evalFormula(adjust[name] ?? fmla, vars))
  for (const [name, fmla] of def.gd) vars.set(name, evalFormula(fmla, vars))
  const val = (a: string | number) => (typeof a === 'number' ? a : Number.isFinite(Number(a)) ? Number(a) : vars.get(a) ?? 0)
  const out: GeomPath[] = []
  for (const path of def.paths) {
    const sx = path.w ? w / path.w : 1
    const sy = path.h ? h / path.h : 1
    const X = (x: number) => (flipH ? w - x * sx : x * sx)
    const Y = (y: number) => (flipV ? h - y * sy : y * sy)
    const mirrored = flipH !== flipV
    const parts: string[] = []
    let cx = 0, cy = 0, startX = 0, startY = 0
    for (const cmd of path.cmds) {
      const [c, ...a] = cmd
      const p = a.map(val)
      switch (c) {
        case 'M': cx = p[0]; cy = p[1]; startX = cx; startY = cy; parts.push(`M${n2(X(cx))} ${n2(Y(cy))}`); break
        case 'L': cx = p[0]; cy = p[1]; parts.push(`L${n2(X(cx))} ${n2(Y(cy))}`); break
        case 'Q': parts.push(`Q${n2(X(p[0]))} ${n2(Y(p[1]))} ${n2(X(p[2]))} ${n2(Y(p[3]))}`); cx = p[2]; cy = p[3]; break
        case 'C': parts.push(`C${n2(X(p[0]))} ${n2(Y(p[1]))} ${n2(X(p[2]))} ${n2(Y(p[3]))} ${n2(X(p[4]))} ${n2(Y(p[5]))}`); cx = p[4]; cy = p[5]; break
        case 'Z': parts.push('Z'); cx = startX; cy = startY; break
        case 'A': {
          const rx = p[0], ry = p[1]
          const st = (p[2] / DEG) * (Math.PI / 180), sw = (p[3] / DEG) * (Math.PI / 180)
          if (rx <= 0 || ry <= 0 || sw === 0) break
          const t0 = Math.atan2(rx * Math.sin(st), ry * Math.cos(st))
          const t1raw = Math.atan2(rx * Math.sin(st + sw), ry * Math.cos(st + sw))
          let dt = t1raw - t0
          if (Math.abs(sw) >= 2 * Math.PI - 1e-9) dt = Math.sign(sw) * 2 * Math.PI
          else if (sw > 0 && dt <= 0) dt += 2 * Math.PI
          else if (sw < 0 && dt >= 0) dt -= 2 * Math.PI
          const ox = cx - rx * Math.cos(t0), oy = cy - ry * Math.sin(t0)
          // a full turn is two halves (an SVG arc cannot end where it starts)
          const steps = Math.abs(dt) > Math.PI * 1.5 ? 2 : 1
          for (let k = 1; k <= steps; k++) {
            const t = t0 + (dt * k) / steps
            const ex = ox + rx * Math.cos(t), ey = oy + ry * Math.sin(t)
            const large = Math.abs(dt / steps) > Math.PI ? 1 : 0
            const sweep = (dt > 0) !== mirrored ? 1 : 0
            parts.push(`A${n2(rx * sx)} ${n2(ry * sy)} 0 ${large} ${sweep} ${n2(X(ex))} ${n2(Y(ey))}`)
            cx = ex; cy = ey
          }
          break
        }
      }
    }
    if (parts.length) out.push({ d: parts.join(''), fill: path.fill ?? 'norm', stroke: path.stroke !== false })
  }
  return out
}

/** A shape's adjust values (`a:avLst/a:gd`). */
export function adjustOf(geom: XmlElement | undefined): Record<string, string> {
  const out: Record<string, string> = {}
  for (const gd of kids(kid(geom, 'avLst'), 'gd')) out[attr(gd, 'name') ?? ''] = attr(gd, 'fmla') ?? ''
  return out
}

export function presetPaths(name: string, w: number, h: number, adjust: Record<string, string>, flipH = false, flipV = false): GeomPath[] | null {
  const def = PRESETS[name]
  return def ? evaluate(def, w, h, adjust, flipH, flipV) : null
}

/** `a:custGeom` read into the preset form (its own guides and paths). */
export function customDef(geom: XmlElement): PresetDef {
  const guides = (el: XmlElement | undefined): [string, string][] => kids(el, 'gd').map((g) => [attr(g, 'name') ?? '', attr(g, 'fmla') ?? 'val 0'])
  const paths: PresetPath[] = []
  for (const path of kids(kid(geom, 'pathLst'), 'path')) {
    const cmds: (string | number)[][] = []
    for (const c of path.children) {
      if (typeof c === 'string') continue
      const pts = kids(c, 'pt').flatMap((p) => [attr(p, 'x') ?? '0', attr(p, 'y') ?? '0'])
      switch (local(c.name)) {
        case 'moveTo': cmds.push(['M', ...pts]); break
        case 'lnTo': cmds.push(['L', ...pts]); break
        case 'quadBezTo': cmds.push(['Q', ...pts]); break
        case 'cubicBezTo': cmds.push(['C', ...pts]); break
        case 'arcTo': cmds.push(['A', attr(c, 'wR') ?? '0', attr(c, 'hR') ?? '0', attr(c, 'stAng') ?? '0', attr(c, 'swAng') ?? '0']); break
        case 'close': cmds.push(['Z']); break
      }
    }
    const fill = attr(path, 'fill')
    const stroke = attr(path, 'stroke')
    paths.push({
      cmds, ...(attr(path, 'w') ? { w: Number(attr(path, 'w')) } : {}), ...(attr(path, 'h') ? { h: Number(attr(path, 'h')) } : {}),
      ...(fill && fill !== 'norm' ? { fill } : {}), ...(stroke === 'false' || stroke === '0' ? { stroke: false } : {}),
    })
  }
  return { av: guides(kid(geom, 'avLst')), gd: guides(kid(geom, 'gdLst')), paths }
}

export const presetNames = () => Object.keys(PRESETS)
