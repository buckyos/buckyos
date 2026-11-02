export interface PdfTextSpan { text: string; x: number; y: number; width: number; size: number }

export interface PdfTextLayout {
  text: string
  width: number
  height: number
  fontSize: number
  align: 'left' | 'center' | 'right'
  lineHeight: number | null
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
}

const record = (value: unknown): Record<string, unknown> | null => value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null
const finite = (value: unknown): value is number => typeof value === 'number' && Number.isFinite(value) && Math.abs(value) <= 1e7
const positive = (value: unknown): value is number => finite(value) && value > 0

export function readPdfTextLayout(config: unknown, look: TextAppearance): PdfTextLayout | null {
  const source = record(record(config)?.pdf_source)
  const layout = record(source?.text_layout)
  if (source?.kind !== 'text' || !layout || look.wrap || look.valign !== 'top' || look.labelPosition !== 'inside' || look.labelFill || layout.text !== look.text || layout.fontSize !== look.fontSize || layout.align !== look.align || layout.lineHeight !== look.lineHeight) return null
  if (!positive(layout.width) || !positive(layout.height) || !positive(layout.fontSize)) return null
  if (layout.align !== 'left' && layout.align !== 'center' && layout.align !== 'right') return null
  if (layout.lineHeight !== null && !positive(layout.lineHeight)) return null
  if (!Array.isArray(layout.spans) || layout.spans.length === 0 || layout.spans.length > 4096) return null
  const spans: PdfTextSpan[] = []
  for (const value of layout.spans) {
    const span = record(value)
    if (!span || typeof span.text !== 'string' || !span.text || !finite(span.x) || !finite(span.y) || !positive(span.width) || !positive(span.size)) return null
    spans.push({ text: span.text, x: span.x, y: span.y, width: span.width, size: span.size })
  }
  if (spans.map((span) => span.text).join('').replace(/\s/g, '') !== look.text.replace(/\s/g, '')) return null
  return { text: look.text, width: layout.width, height: layout.height, fontSize: layout.fontSize, align: layout.align, lineHeight: layout.lineHeight, spans }
}
