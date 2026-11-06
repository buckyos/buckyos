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
The layout stores its base style and per-span overrides for colour, alpha, weight, italic and family.
Repeated overrides are stored once in `styles` and referenced by `styleIndex`. A one-off style can stay inline,
and the reader accepts previously saved inline overrides.
The Desktop validates these fields and renders each override without multiplying source alpha twice.
Span coordinates are relative to the unrotated paragraph box; rotation stays on the native canvas object.
The Desktop draws positioned SVG text with each source advance as `textLength`, so font substitution and
inferred spaces do not move neighbouring fragments. The normal text/config remains editable.

Changing the text removes this source layout. Geometry edits such as font size, alignment or wrapping
switch to the normal layout. Paint edits update just the changed style attributes across the spans and preserve
their source positions and advances. A clamped inspector size still renders the original span sizes. Draw.io
export uses editable HTML fragments with the resolved styles and source text and line breaks.
PDF.js explicit line-end markers prevent adjacent lines from being joined again. Existing imports need
reimporting to acquire the geometry; this does not reconstruct missing metadata from saved canvases.

A paragraph is limited to 4096 spans and 60 KiB of serialized config. If any paragraph exceeds the limit,
the whole page stays in one original raster and the report explains the fallback. No pictures or background
paths are extracted in this case: all source operators retain their paint order, so a separately extracted
background cannot cover the fallback text. Mixed colours/weights are preserved by span overrides. This does not embed the PDF's original fonts or infer semantic word boundaries.

## Browser rendering stream and decoders

The browser captures the complete normal render operator list with `renderOperators` before extracting a page,
then passes that exact list into `extractPage`. The list used by `getOperatorList` can differ because PDF.js disables
its queue optimizer for extraction. Filtered rendering requires the same operator list instance. A mismatch falls back to the complete page;
it must never silently drop the background. Inspection extracts before drawing its preview because CanvasGraphics
may replace raw path buffers with cached Path2D objects while rendering. Deno recording still defaults to
`getOperatorList`, since it does not apply a browser raster filter.

Private-use/replacement glyphs do not become editable text. Their drawing operators must remain in the raster,
including an operator containing both readable and unreadable glyphs. Text items sharing those operators are
retained together. When an extracted shape or picture would cover retained content, the page falls back to one
raster in the original paint order. Optimized repeated images and inline image groups are expanded into separate
objects. Their shared operator is filtered out only after all its members can be extracted.

Import render tasks preserve PDF.js display intent and disable its animation-frame scheduler for the pinned
PDF.js 6.4.299 release. A message task yields between rendering slices so cancellation and the 30-second watchdog can run.
The browser and Deno reader configure the same PDF.js WASM and ICC resources. The Deno reader uses a local
worker for the two synchronous qcms resource reads. Vite serves and emits WASM, JS fallbacks and licences
under `aiworkspace/pdfjs/wasm/` and the DeviceCMYK profile and licence under `aiworkspace/pdfjs/iccs/`, outside the service-worker precache.

The synthetic fixture, browser fidelity checks and result evidence are described in
[PDF fidelity testing](../../../../doc/testing/pdf-import-fidelity.md). No user document is needed.
