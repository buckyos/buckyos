import { stream, writePdf } from './write.ts'

const ccitt = atob('JqC2G///Iq4X/8wGSG1////////////8hscgrOQVnIKzkFZyx///QP//6//9f//r//1//+v/+P//r//r////+v/9f/+v/yQ5h9f/49L8gi9f/+Q2OQVnIKzk5kGtyG13/66//zafBFP/+r6//9Xpf/+/X/+t6//9P1///r//rfX//T6///el//G/X/9Xr//v1//V6/+QyhzD+/X/x/6/+n03pf//9f+nrev/za9P1//f3r/6vpP1/9X96//vq+l/6399f/T++v/r6/X/1f3r/+/v1/63rev/0/BFO+l/63r31/9P1vr//f99f+r676//X131/6t/vr/9ev6X/99d9f+r/vr/9Xr31/+/W+v/revfX/0/W+v+QRev//IbHIPZBRZTkNOyNyDRhCuQfCGW/+sfv1/9Ov6/+vXfX/0/vr//r3pf+r1v1/9Wvev/q9b9f/V/ev/7rv1/69fr/9X7el//r/X/rdd6//X79f+tr3r/9S61v1//2vev/rvW+l/9d/fX/ruu+v/19f1//396/+u679f/XdcR//7+1/69pdf/W/v/69r1/9b1v///1/67a//9fW1/x9+//W6///df/+6//13X//v//12v//vX/6///12v//vX/69//5BF6//8gRhBWchjyBdyCs5BWf/6//Xf////9f//gin2CKH///Ef/1//+v//X//6/////9f//r/////1//+v//X//6//8H//8f////////////////////////////8RER///////////+P/x//4AIAI')

export function fidelityPdf(): Uint8Array {
  const objects = ['', '']
  const add = (body: string) => { objects.push(body); return objects.length }
  const normal = add('<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >>')
  const bold = add('<< /Type /Font /Subtype /Type1 /BaseFont /Times-Bold /Encoding /WinAnsiEncoding >>')
  const italic = add('<< /Type /Font /Subtype /Type1 /BaseFont /Times-Italic /Encoding /WinAnsiEncoding >>')
  const pattern = add(stream('/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 8 8] /XStep 8 /YStep 8 /Resources << >>', '0.8 0.9 0.98 rg 0 0 8 8 re f 0.5 0.72 0.88 rg 0 0 2 2 re f'))
  const image = add(stream('/Type /XObject /Subtype /Image /Width 256 /Height 192 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 256 /Rows 192 /BlackIs1 true >>', ccitt))
  const glyph = add(stream('', '500 0 d0 0 g 40 0 60 800 re f 380 0 60 800 re f 40 740 400 60 re f 40 0 400 60 re f'))
  const unicode = add(stream('', '/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Synthetic) /Ordering (Synthetic) /Supplement 0 >> def /CMapName /SyntheticPrivateUse def /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange 1 beginbfchar <41> <E123> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end'))
  const privateFont = add(`<< /Type /Font /Subtype /Type3 /FontBBox [0 0 500 900] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /Synthetic ${glyph} 0 R >> /Encoding << /Type /Encoding /Differences [65 /Synthetic] >> /FirstChar 65 /LastChar 65 /Widths [500] /Resources << >> /ToUnicode ${unicode} 0 R >>`)
  const background = '/Pattern cs /P1 scn 0 0 720 540 re f 0 g'
  const title = (text: string) => `BT /F2 24 Tf 40 490 Td (${text}) Tj ET`
  const bodies = [
    [background, '0.2 0.4 0.6 rg', ...Array.from({ length: 12 }, (_, i) => `q 1 0 0 1 680 ${440 - i * 26} cm 0 0 12 12 re f Q`), '0 g', title('SYNTHETIC 01 - PATTERN AND OPERATORS'), ...Array.from({ length: 36 }, (_, i) => `BT /F1 14 Tf 1 0 0 1 ${40 + (i % 3) * 210} ${440 - Math.floor(i / 3) * 26} Tm (ROW ${String(i + 1).padStart(2, '0')} - SAMPLE TEXT) Tj ET`)].join('\n'),
    [background, title('SYNTHETIC 02 - CCITT GROUP 4'), 'BT /F1 18 Tf 40 440 Td (Generated grid and zigzag, 256 x 192 pixels.) Tj ET', 'q 384 0 0 288 160 100 cm /Im1 Do Q'].join('\n'),
    [background, title('SYNTHETIC 03 - PRIVATE USE GLYPH'), 'BT /F1 32 Tf 90 330 Td (ALPHA BETA GAMMA) Tj ET', 'BT /G1 32 Tf 88 330 Td (A) Tj ET', 'BT /F1 18 Tf 40 180 Td (The custom outlined glyph maps to U+E123.) Tj ET', 'BT /F1 18 Tf 40 150 Td (Its outline must survive even when text is not editable.) Tj ET'].join('\n'),
    [background, title('SYNTHETIC 04 - MIXED STYLES'), 'BT /F1 24 Tf 40 350 Td (PLAIN ) Tj /F2 24 Tf 0.85 0.05 0.05 rg (RED BOLD ) Tj /F3 24 Tf 0.05 0.15 0.85 rg (BLUE ITALIC) Tj ET', '0 g BT /F1 18 Tf 40 220 Td (All words and artwork in this PDF are generated test data.) Tj ET'].join('\n'),
  ]
  const pages = bodies.map(body => {
    const content = add(stream('', body))
    return add(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 720 540] /Contents ${content} 0 R /Resources << /Font << /F1 ${normal} 0 R /F2 ${bold} 0 R /F3 ${italic} 0 R /G1 ${privateFont} 0 R >> /Pattern << /P1 ${pattern} 0 R >> /XObject << /Im1 ${image} 0 R >> >> >>`)
  })
  objects[0] = '<< /Type /Catalog /Pages 2 0 R >>'
  objects[1] = `<< /Type /Pages /Kids [${pages.map(p => `${p} 0 R`).join(' ')}] /Count ${pages.length} >>`
  return writePdf(objects)
}

if (import.meta.main) {
  const bytes = fidelityPdf()
  await Deno.writeFile(new URL('../synthetic-pdf-import.pdf', import.meta.url), bytes)
  console.log(`synthetic-pdf-import.pdf: ${bytes.length} bytes, 4 synthetic pages`)
}
