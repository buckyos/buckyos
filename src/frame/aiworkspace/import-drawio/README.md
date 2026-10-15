# import-drawio — draw.io import/export and shared canvas geometry

Implements [notepads/aiworkspace-drawio-import-todo.md](../../../../notepads/aiworkspace-drawio-import-todo.md). It is a
dependency-free TypeScript module: it uses no DOM and no packages, so the Desktop canvas (Vite alias `@drawio/*`)
and Deno (the aiws program host, tests and the build script) run the same code. Files import each other with
explicit `.ts` extensions.

```text
xml.ts         small XML reader/writer (elements, attributes, CDATA, entities)
bytes.ts       base64, raw/zlib deflate through (De)CompressionStream, PNG text chunks, image sizes
file.ts        readDrawio: .drawio/.xml (plain or compressed pages), bare <mxGraphModel> (clipboard, URL-encoded too),
               .drawio.png (mxfile tEXt/zTXt/iTXt), .drawio.svg (content attribute), <mxlibrary>
style.ts       draw.io styles: parsing, the default stylesheet's named styles, colours
html.ts        HTML labels → text + whole-label bold/italic/underline/colour; partial formatting and lists are reported
geometry.ts    points, rectangles, rotation
anchors.ts     connector anchors (compass sets of rect, ellipse and polygon outlines; own anchors)       — shared with the canvas
connector.ts   connector geometry (ends, routes, controls, labels)                                      — shared with the canvas
shapes.ts      the `shape` Block: kinds, look keys, SVG paths, outlines, anchors                         — shared with the canvas
builtinSvg.ts  draw.io shapes drawn in script (actor, tape, delay, cube…) as SVG shape bodies
stencil.ts     mxStencil XML → SVG shape body (colours as CSS variables) and the `stencil(...)` style value decoder
svg.ts         SVG body cleaning (no scripts, events, foreignObject, external references); pasted SVG → recolourable shape
keys.ts        ids and order keys by the core's rules (`keysAfter` keeps thousands of siblings short)
convert.ts     a draw.io page → import plan (objects, lines, pictures) + report of what was not kept
operations.ts  an import plan → commit operations (objects before lines, parents before children, batches of 2 000)
export.ts      a canvas (Surface BlockTree) → .drawio, starting from the style an import kept in config.drawio
mod.ts         the public entry
tools/build_stencils.ts   builds the library packs in src/frame/desktop/public/aiworkspace/stencils/
```

## Test

```bash
deno test -A                          # convert_test.ts (fixtures, golden plans, operations, round trips), stencil_test.ts
DRAWIO_REGEN=1 deno test -A convert_test.ts   # rewrite fixtures/drawio/expected/*.plan.json after an intended change
deno run -A ../fixtures/drawio/derive.ts      # rebuild the compressed / .png / .svg / library fixtures from fixtures/drawio/src
```

The Desktop e2e suite (`tests/aiworkspace/drawio.spec.ts`) drives the importer, the library, the shapes and the
export through the real app.

## Library packs

`deno run -A tools/build_stencils.ts` downloads `js/stencils.min.js` from jgraph/drawio (branch `dev`, draw.io's default branch).
It decodes its binary stencil sets and converts the shapes of the libraries flowchart, basic, arrows, bpmn and networks into one
JSON pack per library plus `index.json` (about 190 KiB in total). `LICENSE.txt` next to the packs carries draw.io's
Apache-2.0 notice and its stencil notice, which only restricts use in Atlassian products. UML and ER shapes are drawn
in script by draw.io, not by stencil sets. The importer maps the common ones to basic kinds or to `builtinSvg.ts`.
