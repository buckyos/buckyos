/** Appearance shared by a label Block, a projected label and a mind map node's title. */
import contract from './core.json' with { type: 'json' }

export interface LabelLook {
  icon?: string
  icon_position?: 'left' | 'right'
  font_family?: 'sans' | 'serif' | 'mono'
  font_size?: number
  font_color?: string
  weight?: 'normal' | 'medium' | 'bold'
  italic?: boolean
  strike?: boolean
  align?: 'left' | 'center' | 'right'
  valign?: 'top' | 'middle' | 'bottom'
  wrap?: boolean
  max_lines?: number
  mono?: boolean
  muted?: boolean
  background_color?: string
  background_image?: string
}

// Stable document ids. The same glyphs work in a projected DOM label and in standalone SVG.
export const LABEL_ICONS: Readonly<Record<string, string>> = {
  star: '★', heart: '♥', flag: '⚑', target: '◎', idea: '💡', book: '📖', work: '💼', calendar: '📅',
}
export const LABEL_ICON_GAP = 4

export function labelIcon(look: LabelLook): string | undefined {
  return look.icon && Object.hasOwn(LABEL_ICONS, look.icon) ? LABEL_ICONS[look.icon] : undefined
}

export function labelFontFamily(look: LabelLook): string | undefined {
  switch (look.font_family ?? (look.mono ? 'mono' : undefined)) {
    case 'sans': return 'system-ui, sans-serif'
    case 'serif': return 'Georgia, "Noto Serif CJK SC", "Songti SC", SimSun, serif'
    case 'mono': return 'ui-monospace, SFMono-Regular, Menlo, Consolas, "Noto Sans Mono CJK SC", monospace'
  }
}

/** One image URL, shared by CSS backgrounds and SVG image elements. */
export function labelBackgroundImage(value: unknown): string | undefined {
  if (typeof value !== 'string' || !/^https?:\/\//i.test(value)) return undefined
  try { return new URL(value).href } catch { return undefined }
}

/** The outline contains only the appearance properties needed to draw and measure text. */
export function labelConfig(config: unknown): Record<string, string | number | boolean> | undefined {
  if (!config || typeof config !== 'object') return undefined
  const fields: Record<string, string | number | boolean> = {}
  for (const key of contract.label_appearance) {
    const value = (config as Record<string, unknown>)[key]
    if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') fields[key] = value
  }
  return Object.keys(fields).length ? fields : undefined
}

export function readLabelLook(config: unknown): LabelLook {
  if (!config || typeof config !== 'object') return {}
  const look = config as LabelLook
  if (look.font_size === undefined || (typeof look.font_size === 'number' && Number.isFinite(look.font_size) && look.font_size > 0)) return look
  return { ...look, font_size: undefined }
}
