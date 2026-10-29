/* PDFs the PDF import tests make (tests/aiworkspace/pdf-import.spec.ts, tests/dv/aiworkspace.pdf.spec.ts). */

/** A PDF from object bodies (Latin-1; stream lengths computed). */
function writePdf(objects: string[]): Buffer {
  let text = '%PDF-1.5\n'
  const offsets: number[] = []
  objects.forEach((body, i) => { offsets.push(text.length); text += `${i + 1} 0 obj\n${body}\nendobj\n` })
  const xref = text.length
  text += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map((o) => `${String(o).padStart(10, '0')} 00000 n \n`).join('')}`
  text += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`
  return Buffer.from(text, 'latin1')
}
const stream = (dict: string, data: string) => `<< ${dict} /Length ${data.length} >>\nstream\n${data}\nendstream`

/** Three pages: 720 × 405 (16:9), 720 × 405, 595 × 842 (A4 portrait). Page 1: a navy background, white bold and
 * regular text, an orange box, an 8 × 8 picture stretched to 120 × 90; page 2: a heading, Chinese text through a
 * predefined CMap with a font the file does not embed, a green rule; page 3: text and a box drawn with the Multiply
 * blend mode (it stays in the page's raster). */
export function deck(): Buffer {
  let pixels = ''
  for (let y = 0; y < 8; y++) for (let x = 0; x < 8; x++) pixels += String.fromCharCode(40 + x * 25, 200 - y * 20, 120)
  const p1 = [
    '0.12 0.23 0.37 rg 0 0 720 405 re f',
    '1 1 1 rg BT /F1 36 Tf 48 330 Td (Quarterly Review) Tj ET',
    'BT /F2 18 Tf 48 280 Td (Revenue grew 23 percent) Tj 0 -24 Td (New customers: 1,204) Tj ET',
    '0.96 0.62 0.04 rg 420 120 220 140 re f',
    'q 120 0 0 90 48 60 cm /Im1 Do Q',
  ].join('\n')
  const p2 = [
    '0 g BT /F1 30 Tf 48 330 Td (Roadmap) Tj ET',
    // 中文用例 in UCS-2 through a predefined CMap, the font not embedded (pdf.js needs the CMap file)
    'BT /F3 24 Tf 48 260 Td <4E2D658775284F8B> Tj ET',
    '0.2 0.6 0.3 RG 3 w 48 200 m 600 200 l S',
  ].join('\n')
  const p3 = '0 g BT /F2 14 Tf 60 760 Td (A portrait page in the same deck.) Tj ET\nq /GS1 gs 0.9 0.2 0.2 rg 60 500 240 160 re f Q'
  const res = '/Font << /F1 9 0 R /F2 10 0 R /F3 11 0 R >> /XObject << /Im1 12 0 R >> /ExtGState << /GS1 << /Type /ExtGState /BM /Multiply >> >>'
  return writePdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>',
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 720 405] /Contents 6 0 R /Resources << ${res} >> >>`,
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 720 405] /Contents 7 0 R /Resources << ${res} >> >>`,
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 8 0 R /Resources << ${res} >> >>`,
    stream('', p1),
    stream('', p2),
    stream('', p3),
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
    '<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light /Encoding /UniGB-UCS2-H /DescendantFonts [13 0 R] >>',
    stream('/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceRGB /BitsPerComponent 8', pixels),
    '<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 2 >> /FontDescriptor 14 0 R >>',
    '<< /Type /FontDescriptor /FontName /STSong-Light /Flags 6 /FontBBox [0 -200 1000 900] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 880 /StemV 80 >>',
  ])
}
