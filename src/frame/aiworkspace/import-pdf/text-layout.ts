import { clampShapeFontSize } from '../import-drawio/shapes.ts'

export interface PdfTextStyle { color: string; alpha: number; bold: boolean; italic: boolean; family: 'sans' | 'serif' | 'mono' }

export interface PdfTextSpan { style?: PdfTextStyle; text: string; x: number; y: number; width: number; size: number }

export interface PdfTextLayout {
  text: string
  width: number
  height: number
  fontSize: number
  align: 'left' | 'center' | 'right'
  lineHeight: number | null
  style?: PdfTextStyle
  spans: PdfTextSpan[]
}

interface TextAppearance {
  text: string
  fontSize: number
  align: string
  lineHeight: number | null
  wrap: boolean
  valign: string
  labelPosition: string
  labelFill: string | null
  fontColor: string | null
  bold: boolean
  italic: boolean
  fontFamily: string | null
  opacity: number
}

const record = (value: unknown): Record<string, unknown> | null => value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null
const finite = (value: unknown): value is number => typeof value === 'number' && Number.isFinite(value) && Math.abs(value) <= 1e7
const positive = (value: unknown): value is number => finite(value) && value > 0

function readStyle(value: unknown): PdfTextStyle | null {
  const style = record(value)
  if (!style || typeof style.color !== 'string' || !/^#[0-9a-f]{6}$/i.test(style.color) || !finite(style.alpha) || style.alpha < 0 || style.alpha > 1 || typeof style.bold !== 'boolean' || typeof style.italic !== 'boolean' || (style.family !== 'sans' && style.family !== 'serif' && style.family !== 'mono')) return null
  return { color: style.color, alpha: style.alpha, bold: style.bold, italic: style.italic, family: style.family as PdfTextStyle['family'] }
}

export function readPdfTextLayout(config: unknown, look: TextAppearance): PdfTextLayout | null {
  const source = record(record(config)?.pdf_source)
  const layout = record(source?.text_layout)
  if (source?.kind !== 'text' || !layout || look.wrap || look.valign !== 'top' || look.labelPosition !== 'inside' || look.labelFill || layout.text !== look.text || layout.align !== look.align || layout.lineHeight !== look.lineHeight) return null
  if (!positive(layout.width) || !positive(layout.height) || !positive(layout.fontSize) || clampShapeFontSize(layout.fontSize) !== look.fontSize) return null
  if (layout.align !== 'left' && layout.align !== 'center' && layout.align !== 'right') return null
  if (layout.lineHeight !== null && !positive(layout.lineHeight)) return null
  if (!Array.isArray(layout.spans) || layout.spans.length === 0 || layout.spans.length > 4096) return null
  const style = layout.style === undefined ? undefined : readStyle(layout.style)
  if (style === null) return null
  const styles: PdfTextStyle[] = []
  if (layout.styles !== undefined) {
    if (!style || !Array.isArray(layout.styles) || layout.styles.length > layout.spans.length) return null
    for (const value of layout.styles) {
      const entry = readStyle(value)
      if (!entry) return null
      styles.push(entry)
    }
  }
  const edits: Partial<PdfTextStyle> = {}
  if (style) {
    const color = look.fontColor && look.fontColor !== 'auto' ? look.fontColor : ''
    const family = look.fontFamily === 'mono' || look.fontFamily === 'serif' ? look.fontFamily : 'sans'
    if (style.color.toLowerCase() !== color.toLowerCase()) edits.color = color
    if (style.alpha !== look.opacity) edits.alpha = look.opacity
    if (style.bold !== look.bold) edits.bold = look.bold
    if (style.italic !== look.italic) edits.italic = look.italic
    if (style.family !== family) edits.family = family
  }
  const spans: PdfTextSpan[] = []
  for (const value of layout.spans) {
    const span = record(value)
    if (!span || typeof span.text !== 'string' || !span.text || !finite(span.x) || !finite(span.y) || !positive(span.width) || !positive(span.size)) return null
    let spanStyle = span.style === undefined ? undefined : readStyle(span.style)
    if (span.styleIndex !== undefined) {
      if (span.style !== undefined || typeof span.styleIndex !== 'number' || !Number.isSafeInteger(span.styleIndex)) return null
      spanStyle = styles[span.styleIndex]
      if (!spanStyle) return null
    }
    if (spanStyle === null || (spanStyle && !style)) return null
    spans.push({ ...(spanStyle ? { style: { ...spanStyle, ...edits } } : {}), text: span.text, x: span.x, y: span.y, width: span.width, size: span.size })
  }
  if (spans.map((span) => span.text).join('').replace(/\s/g, '') !== look.text.replace(/\s/g, '')) return null
  return { text: look.text, width: layout.width, height: layout.height, fontSize: layout.fontSize, align: layout.align, lineHeight: layout.lineHeight, ...(style ? { style: { ...style, ...edits } } : {}), spans }
}
