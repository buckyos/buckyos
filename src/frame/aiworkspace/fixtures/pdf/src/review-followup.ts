import { stream, writePdf } from './write.ts'

function makeDocument() {
  const objects = ['', '']
  const add = (body: string) => { objects.push(body); return objects.length }
  const finish = (bodies: string[], resources: string, box = '0 0 720 540') => {
    const pages = bodies.map(body => {
      const content = add(stream('', body))
      return add(`<< /Type /Page /Parent 2 0 R /MediaBox [${box}] /Contents ${content} 0 R /Resources << ${resources} >> >>`)
    })
    objects[0] = '<< /Type /Catalog /Pages 2 0 R >>'
    objects[1] = `<< /Type /Pages /Kids [${pages.map(p => `${p} 0 R`).join(' ')}] /Count ${pages.length} >>`
    return writePdf(objects)
  }
  return { add, finish }
}

export function reviewFollowupPdf(): Uint8Array {
  const { add, finish } = makeDocument()
  const font = (name: string, extra = '') => add(`<< /Type /Font /Subtype /Type1 /BaseFont /${name} /Encoding /WinAnsiEncoding ${extra} >>`)
  const normal = font('Times-Roman'), bold = font('Times-Bold'), italic = font('Times-Italic'), hidden = font('Courier')
  const unicode = add(stream('', [
    '/CIDInit /ProcSet findresource begin 12 dict begin begincmap',
    '/CIDSystemInfo << /Registry (Synthetic) /Ordering (Synthetic) /Supplement 0 >> def',
    '/CMapName /SyntheticPrivate def /CMapType 2 def',
    '1 begincodespacerange <00> <FF> endcodespacerange',
    '1 beginbfchar <7E> <E123> endbfchar',
    'endcmap CMapName currentdict /CMap defineresource pop end end',
  ].join('\n')))
  const privateFont = font('Times-Roman', `/ToUnicode ${unicode} 0 R`)
  const image = (color: string) => add(stream('/Type /XObject /Subtype /Image /Width 16 /Height 16 /ColorSpace /DeviceRGB /BitsPerComponent 8', color.repeat(256)))
  const red = image('\xff\x00\x00'), yellow = image('\xff\xff\x00')
  const tiny = [
    'BT /F1 1.25 Tf 48 420 Td (TINY ) Tj /F2 1.25 Tf 0.85 0.05 0.05 rg (RED ) Tj /F3 1.25 Tf 0.05 0.15 0.85 rg (BLUE) Tj ET',
    '0 g BT /F1 18 Tf 48 300 Td (VISIBLE) Tj ET',
    'BT /F4 18 Tf 48 300 Td 3 Tr (HIDDEN OCR WORDS) Tj ET',
  ].join('\n')
  const dense = Array.from({ length: 12 }, (_, row) => [
    'BT',
    ...Array.from({ length: 64 }, (_, col) => {
      const paint = col % 2 ? '/F2 3 Tf 0.85 0.05 0.05 rg' : '/F1 3 Tf 0 g'
      return `${paint} 1 0 0 1 ${34 + col * 4.5} ${480 - row * 4.7} Tm (A) Tj`
    }),
    'ET',
  ].join('\n')).join('\n')
  const layered = [
    'q 40 190 270 170 re W n',
    'q 450 0 0 170 40 190 cm /Red Do Q',
    ...[70, 130, 190, 250, 370].map(x => `q 30 0 0 30 ${x} 260 cm /Yellow Do Q`),
    'Q BT /F5 22 Tf 64 90 Td (~~~~~~~~~~~~) Tj ET',
  ].join('\n')
  return finish([tiny, dense, layered], `/Font << /F1 ${normal} 0 R /F2 ${bold} 0 R /F3 ${italic} 0 R /F4 ${hidden} 0 R /F5 ${privateFont} 0 R >> /XObject << /Red ${red} 0 R /Yellow ${yellow} 0 R >>`)
}

export function syntheticCmykPdf(profile: Uint8Array): Uint8Array {
  const { add, finish } = makeDocument()
  const embedded = add(stream('/N 4 /Alternate /DeviceCMYK', Array.from(profile, byte => String.fromCharCode(byte)).join('')))
  return finish([
    '0.2 0.7 0.1 0.25 k 0 0 100 100 re f /Embedded cs 0.2 0.7 0.1 0.25 scn 100 0 100 100 re f',
  ], `/ColorSpace << /Embedded [/ICCBased ${embedded} 0 R] >>`, '0 0 200 100')
}

export function syntheticLongRenderPdf(): Uint8Array {
  const { finish } = makeDocument()
  const body = Array.from({ length: 80_000 }, (_, i) => `${i % 2 ? '0.5 0.1 0.8' : '0.1 0.7 0.9'} rg ${i % 360 * 2} ${Math.floor(i / 360) * 2} 3 3 re f`).join('\n')
  return finish([body], '')
}
