/* Record the pages of a PDF as converter input (`deno run -A tools/record.ts in.pdf out.pages.json`): the JSON the
 * unit tests convert without pdf.js. Picture pixels are not recorded (the tests stub the host's encoder). */

import { extractPage, readBookmarks } from '../extract.ts'
import { openPdf } from './pdfjs.ts'

const [input, output] = Deno.args
if (!input || !output) { console.error('usage: record.ts <in.pdf> <out.pages.json>'); Deno.exit(2) }
const { doc, oc } = await openPdf(await Deno.readFile(input))
const bookmarks = await readBookmarks(doc)
const pages = []
for (let n = 1; n <= doc.numPages; n++) pages.push((await extractPage(doc, await doc.getPage(n), bookmarks, oc)).page)
await Deno.writeTextFile(output, JSON.stringify({ pages }) + '\n')
console.log(`${input}: ${pages.length} pages → ${output}`)
Deno.exit(0)
