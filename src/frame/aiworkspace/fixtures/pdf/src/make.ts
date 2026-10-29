/* Hand-written fixtures (`deno run -A make.ts`): `latex.pdf`, an untagged two-page article set like LaTeX output
 * (standard Times fonts, justified paragraphs with a first-line indent, kerned TJ arrays, a rule, a boxed figure
 * with a circle and a line, a rotated label, a centred page number), and `scan.pdf`, a scanned page (one picture
 * over the page, the recognised text invisible on top). `slides.pdf` and `doc.pdf` are printed by Chromium:
 * `node print.mjs` (see there). */

import { stream, writePdf } from './write.ts'

const dir = new URL('../', import.meta.url)

function latex(): Uint8Array {
  const body = (lines: string[], x: number, y: number, lead: number, indent: number) =>
    lines.map((l, i) => `BT /F1 10 Tf ${i === 0 ? x + indent : x} ${y - i * lead} Td [${l}] TJ ET`).join('\n')
  const para1 = [
    '(This is the first paragraph of a short article, typeset the way) -250 (LaTeX does it.)',
    '(Every line is justified: the space between words varies from line) -120 (to line, and kerning)',
    '(moves single letters by a few thousandths of an em, as in) -333 (W) 80 (ord and) -333 (T) 70 (ype.)',
    '(The last line of a paragraph is shorter.)',
  ]
  const para2 = [
    '(A second paragraph starts with the same indent and runs over two lines so) -300 (that the)',
    '(importer has to keep it apart from the first one.)',
  ]
  const p1 = [
    'BT /F2 17 Tf 186 700 Td (A Short Article on Import) Tj ET',
    'BT /F1 12 Tf 262 676 Td (A. N. Author) Tj ET',
    'BT /F2 12 Tf 72 640 Td (1  Introduction) Tj ET',
    body(para1, 72, 620, 12, 15),
    body(para2, 72, 566, 12, 15),
    'BT /F3 10 Tf 230 530 Td (E = mc) Tj 0 4 Td /F1 7 Tf (2) Tj ET',
    '0 0 0 RG 0.4 w 72 512 m 540 512 l S',
    // a figure: a box, a circle, a diagonal line, a caption, a rotated label
    '0.85 0.9 1 rg 0.2 0.3 0.6 RG 1 w 150 340 300 150 re B',
    '1 0.6 0 rg 252 415 m 252 437.1 234.1 455 212 455 c 189.9 455 172 437.1 172 415 c 172 392.9 189.9 375 212 375 c 234.1 375 252 392.9 252 415 c f',
    '0 0.5 0 RG 2 w [6 3] 0 d 270 360 m 430 470 l S [] 0 d',
    '0 g BT /F1 9 Tf 0 1 -1 0 140 380 Tm (vertical axis) Tj ET',
    'BT /F3 10 Tf 225 320 Td (Figure 1: A box, a circle and a line.) Tj ET',
    'BT /F1 8 Tf 72 90 Td (1) Tj 6 0 Td (A footnote in a smaller size.) Tj ET',
    'BT /F1 10 Tf 303 50 Td (1) Tj ET',
  ].join('\n')
  const p2 = [
    'BT /F2 12 Tf 72 700 Td (2  Results) Tj ET',
    body(['(The second page has one more paragraph and a table drawn with) -200 (rules.)', '(Its cells are separate lines of text.)'], 72, 680, 12, 15),
    '0 0 0 RG 0.6 w 72 640 m 300 640 l S 72 600 m 300 600 l S 72 620 m 300 620 l S',
    'BT /F2 10 Tf 76 626 Td (Name) Tj 120 0 Td (Value) Tj ET',
    'BT /F1 10 Tf 76 606 Td (Pages) Tj 120 0 Td (2) Tj ET',
    'BT /F1 10 Tf 303 50 Td (2) Tj ET',
  ].join('\n')
  return writePdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R /Resources << /Font << /F1 7 0 R /F2 8 0 R /F3 9 0 R >> >> >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 6 0 R /Resources << /Font << /F1 7 0 R /F2 8 0 R /F3 9 0 R >> >> >>',
    stream('', p1),
    stream('', p2),
    '<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Times-Bold /Encoding /WinAnsiEncoding >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Times-Italic /Encoding /WinAnsiEncoding >>',
  ])
}

function scan(): Uint8Array {
  // a 16 × 16 grey "photo" stretched over the page; the recognised text drawn invisible (3 Tr) over it
  let pixels = ''
  for (let y = 0; y < 16; y++) for (let x = 0; x < 16; x++) { const v = 200 + ((x * 3 + y * 5) % 40); pixels += String.fromCharCode(v, v, v - 10) }
  const content = [
    'q 595 0 0 842 0 0 cm /Im1 Do Q',
    'BT 3 Tr /F1 14 Tf 72 760 Td (Scanned page title) Tj 0 -20 Td (Some recognised words under the picture.) Tj ET',
  ].join('\n')
  return writePdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> /XObject << /Im1 6 0 R >> >> >>',
    stream('', content),
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
    stream('/Type /XObject /Subtype /Image /Width 16 /Height 16 /ColorSpace /DeviceRGB /BitsPerComponent 8', pixels),
  ])
}

await Deno.writeFile(new URL('latex.pdf', dir), latex())
await Deno.writeFile(new URL('scan.pdf', dir), scan())
console.log('latex.pdf, scan.pdf written')
