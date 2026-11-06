import { stream, writePdf } from './write.ts'

export function reviewPdf(): Uint8Array {
  const objects = ['', '']
  const add = (body: string) => { objects.push(body); return objects.length }
  const unicode = add(stream('', [
    '/CIDInit /ProcSet findresource begin 12 dict begin begincmap',
    '/CIDSystemInfo << /Registry (Synthetic) /Ordering (Synthetic) /Supplement 0 >> def',
    '/CMapName /SyntheticMixedRun def /CMapType 2 def',
    '1 begincodespacerange <00> <FF> endcodespacerange',
    '1 beginbfchar <7E> <E123> endbfchar',
    'endcmap CMapName currentdict /CMap defineresource pop end end',
  ].join('\n')))
  const font = add(`<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding /ToUnicode ${unicode} 0 R >>`)
  const icon = add(stream('/Type /XObject /Subtype /Image /Width 16 /Height 16 /ColorSpace /DeviceRGB /BitsPerComponent 8', '\xf4\xc1\x19'.repeat(256)))
  const mixedRun = 'BT /F1 56 Tf 72 325 Td (ABCDEFGH) Tj (~IJ) Tj ET'
  const inline = (x: number, color: string) => `q 34 0 0 34 ${x} 168 cm BI /W 2 /H 2 /CS /RGB /BPC 8 ID ${color.repeat(4)}\nEI Q`
  const bodies = [
    `0.12 0.24 0.52 rg 40 240 590 150 re f 1 g BT /F1 24 Tf 72 325 Td (ABCDEFGH~IJ) Tj ET`,
    `0 g ${mixedRun} BT /F1 20 Tf 72 192 Td (STAYS EDITABLE) Tj ET 3 Tr BT /F1 18 Tf 72 96 Td (~) Tj ET`,
    ['0.12 0.24 0.52 rg 40 125 600 270 re f',
      ...[78, 158, 238, 318].map(x => `q 42 0 0 42 ${x} 268 cm /Im1 Do Q`),
      ...Array.from({ length: 10 }, (_, i) => inline(75 + 48 * i, i % 2 ? '\xff\x55\x40' : '\xf4\xc1\x19')),
    ].join('\n'),
  ]
  const pages = bodies.map(body => {
    const content = add(stream('', body))
    return add(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 720 540] /Contents ${content} 0 R /Resources << /Font << /F1 ${font} 0 R >> /XObject << /Im1 ${icon} 0 R >> >> >>`)
  })
  objects[0] = '<< /Type /Catalog /Pages 2 0 R >>'
  objects[1] = `<< /Type /Pages /Kids [${pages.map(p => `${p} 0 R`).join(' ')}] /Count ${pages.length} >>`
  return writePdf(objects)
}
