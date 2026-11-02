import { stream, writePdf } from './write.ts'

export function overflowTextPdf(): Uint8Array {
  const operators = ['0.6 g 10 100 440 680 re f', '0 g']
  for (let row = 0; row < 40; row++) {
    for (let col = 0; col < 30; col++) {
      operators.push(`BT /F${col % 2 + 1} 12 Tf 1 0 0 1 ${20 + col * 12} ${760 - row * 16} Tm (x) Tj ET`)
    }
  }
  return writePdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Contents 4 0 R /Resources << /Font << /F1 5 0 R /F2 6 0 R >> >> >>',
    stream('', operators.join('\n')),
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Oblique >>',
  ])
}
