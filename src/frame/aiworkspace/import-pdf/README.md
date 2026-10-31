# import-pdf — PDF pages as editable canvas objects

Implements [PDF 导入与放映方案](../../../../doc/workspace/BuckyOS%20AI%20Workspace%20PDF%20导入与放映方案讨论.md) §3–§5. A
dependency-free TypeScript module like `../import-drawio` (whose `PagePlan`, `buildOperations` and `ImportReport` it
reuses): pdf.js is handed in, never imported, so the Desktop (Vite alias `@pdf/*`) and Deno run the same code. Files
import each other with explicit `.ts` extensions.

```text
data.ts        the page data the converter reads (JSON) and pdf.js 6.4's op codes
extract.ts     pdf.js document / page → that data (text with marked content, compacted operator list, fonts,
               shadings, structure tree, links, bookmarks); pictures stay with pdf.js, referenced by id
interpret.ts   one walk over the operator list: graphics state, text runs, paths, pictures, clips, in world units
text.ts        items + runs → lines → paragraphs (structure tree blocks, or geometry), box and line height
vector.ts      paths → rect / line / ellipse / SVG shapes (joined, clipped, gradients), SVG size limit
convert.ts     a page → import plan: group + covering Frame (page colour, `presentation.background`), raster,
               objects in paint order; the fallbacks (unreadable text, too many paths, high-fidelity mode)
layout.ts      page scale (long side 1920), grid of pages, stage size
mod.ts         the public entry
tools/record.ts   record a PDF's pages as test input (uses the Desktop's pdfjs-dist)
tools/pdfjs.ts    pdf.js for Deno (the Desktop's copy, legacy build)
```

The host does what needs pixels: `encodeImage` turns a picture reference into PNG / JPEG bytes and `renderRaster`
renders the page without the operators that became objects (pdf.js `operationsFilter`), cropped to what is left.

## Test

```bash
deno test -A                                   # convert_test.ts (golden plans, rules), extract_test.ts (pdf.js in sync)
PDF_REGEN=1 deno test -A convert_test.ts       # rewrite ../fixtures/pdf/expected/*.plan.json after an intended change
deno run -A tools/record.ts ../fixtures/pdf/latex.pdf ../fixtures/pdf/latex.pages.json   # re-record a fixture
cd ../fixtures/pdf/src && deno run -A make.ts && node print.mjs                          # rebuild the fixture PDFs
```

`extract_test.ts` needs `src/frame/desktop/node_modules` (pdfjs-dist 6.4.299) and is skipped without it. The Desktop
e2e suite (`tests/aiworkspace/pdf-import.spec.ts`) drives the importer through the real app.

## Editable text layout

PDF text objects keep `config.pdf_source.text_layout`: the original text, paragraph box width/height,
font size, alignment, line height, and `spans` with `text`, `x`, `y` (baseline), `width`, and `size`.
Span coordinates are relative to the unrotated paragraph box; rotation stays on the native canvas object.
The Desktop draws positioned SVG text with each source advance as `textLength`, so font substitution and
inferred spaces do not move neighbouring fragments. The normal text/config remains editable.

Changing the text removes this source layout. A text, font-size, alignment, line-height, wrapping, vertical-alignment, label-position, or label-background
mismatch disables it, as does malformed metadata. Ordinary text objects use their normal renderer.
PDF.js explicit line-end markers prevent adjacent lines from being joined again. Existing imports need
reimporting to acquire the geometry; this does not reconstruct missing metadata from saved canvases.

A paragraph is limited to 4096 spans and 60 KiB of serialized config. If any paragraph exceeds the limit,
the page's text stays in the original raster and the report explains the fallback, preserving the source
operators rather than writing an invalid config or silently dropping text. Mixed colours/weights still
use the paragraph's main style. This does not embed the PDF's original fonts or infer semantic word boundaries.
