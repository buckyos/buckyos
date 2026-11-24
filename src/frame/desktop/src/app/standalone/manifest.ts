/* ── system-apps.json and the icons it names (vite.config.ts writes them) ──
 *
 * What the BuckyOS client needs to list a Zone's system apps, from the same catalog the desktop shows. Runs in Node
 * while the desktop is built: the icons are images that depend on nothing, so the colors are resolved here.
 */

import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { LayoutGrid } from 'lucide-react'
import type { AppWindowDeclaration } from 'buckyos/open-app'
import { appIcons } from '../../components/appIcons'
import { dictionaries } from '../../i18n/dictionaries'
import type { AppDefinition, FormFactor } from '../../models/ui'
import { isLauncherApp, supportedLocales, supportsFormFactor } from '../../models/ui'
import { windowDeclarationOf } from '../windowDeclaration'
import { systemAppAddress } from './address'
import { systemAppAudience, systemApps, type SystemAppAudience } from './catalog'

export interface SystemAppEntry {
  /** `OpenAppRequest.app`. */
  id: string
  names: Record<string, string>
  icon: string
  path: string
  show_in_launcher: boolean
  audience: SystemAppAudience
  form_factors: FormFactor[]
  window: AppWindowDeclaration
}

export interface SystemAppsManifest { version: 1; apps: SystemAppEntry[] }

const ICON_DIR = '/system-apps/'
const SIZE = 128

type Rgb = [number, number, number]

/** sRGB (0–1) of an `oklch(L C H)` value. */
function oklchToRgb(value: string): Rgb {
  const match = /^oklch\(\s*([\d.]+)\s+([\d.]+)\s+([\d.]+)\s*\)$/.exec(value.trim())
  if (!match) throw new Error(`system apps: cannot read the color ${value}`)
  const [l, c, h] = match.slice(1).map(Number)
  const a = c * Math.cos((h * Math.PI) / 180)
  const b = c * Math.sin((h * Math.PI) / 180)
  const [lc, mc, sc] = [l + 0.3963377774 * a + 0.2158037573 * b, l - 0.1055613458 * a - 0.0638541728 * b, l - 0.0894841775 * a - 1.291485548 * b].map((x) => x ** 3)
  const linear = [
    4.0767416621 * lc - 3.3077115913 * mc + 0.2309699292 * sc,
    -1.2684380046 * lc + 2.6097574011 * mc - 0.3413193965 * sc,
    -0.0041960863 * lc - 0.7034186147 * mc + 1.707614701 * sc,
  ]
  return linear.map((x) => {
    const clipped = Math.min(1, Math.max(0, x))
    return clipped <= 0.0031308 ? 12.92 * clipped : 1.055 * clipped ** (1 / 2.4) - 0.055
  }) as Rgb
}

const hex = (rgb: Rgb) => `#${rgb.map((x) => Math.round(x * 255).toString(16).padStart(2, '0')).join('')}`
/** As `color-mix(in srgb, <color> 90%, white)`, the top of an icon tile (DesktopVisualTokens.ts). */
const lighter = (rgb: Rgb) => rgb.map((x) => x * 0.9 + 0.1) as Rgb

function iconSvg(app: AppDefinition, tokens: Readonly<Record<string, string>>): string {
  const token = /^var\((--[\w-]+)\)$/.exec(app.accent)?.[1]
  const color = oklchToRgb((token && tokens[token]) ?? app.accent)
  // The shapes of the icon without its own `svg` element, which not every renderer takes inside another one.
  const icon = renderToStaticMarkup(createElement(appIcons[app.iconKey as keyof typeof appIcons] ?? LayoutGrid))
  const shapes = icon.replace(/^<svg[^>]*>|<\/svg>$/g, '').replaceAll('currentColor', '#ffffff')
  const glyph = `<g transform="translate(${SIZE / 4} ${SIZE / 4}) scale(${SIZE / 2 / 24})" fill="none" stroke="#ffffff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${shapes}</g>`
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${SIZE} ${SIZE}" width="${SIZE}" height="${SIZE}"><defs><linearGradient id="tile" x1="0" y1="0" x2="0" y2="1"><stop stop-color="${hex(lighter(color))}"/><stop offset="1" stop-color="${hex(color)}"/></linearGradient></defs><rect width="${SIZE}" height="${SIZE}" rx="${SIZE * 0.28}" fill="url(#tile)"/>${glyph}</svg>`
}

export interface SystemAppsInput {
  /** Values of the `--cp-app-*` tokens. */
  tokens: Readonly<Record<string, string>>
  /** The language packs that are files of their own (src/i18n/locales). */
  languagePacks: Readonly<Record<string, Readonly<Record<string, string>>>>
  /** Apps whose icon is an image of its own. */
  images: Readonly<Record<string, string>>
}

/** Published path → content. */
export function systemAppsFiles(input: SystemAppsInput): Record<string, string> {
  const apps = systemApps()
  const manifest: SystemAppsManifest = {
    version: 1,
    apps: apps.map((app) => ({
      id: app.id,
      names: Object.fromEntries(supportedLocales.map((locale) => [locale, (input.languagePacks[locale] ?? dictionaries[locale])[app.labelKey] ?? dictionaries.en[app.labelKey]])),
      icon: `${ICON_DIR}${app.id}.svg`,
      path: systemAppAddress(app.id),
      show_in_launcher: isLauncherApp(app),
      audience: systemAppAudience(app.id),
      form_factors: (['desktop', 'mobile'] as const).filter((formFactor) => supportsFormFactor(app, formFactor)),
      window: windowDeclarationOf(app.manifest),
    })),
  }
  return {
    'system-apps.json': `${JSON.stringify(manifest, null, 2)}\n`,
    ...Object.fromEntries(apps.map((app) => [`${ICON_DIR.slice(1)}${app.id}.svg`, input.images[app.id] ?? iconSvg(app, input.tokens)])),
  }
}
