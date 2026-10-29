/* PDF import (PDF 导入方案): read pages out of pdf.js, convert each into an import plan the draw.io importer's
 * `buildOperations` writes, lay the pages out on one canvas. Shared by the Desktop and Deno; no DOM, no packages. */

export { OPS, DRAW, type PdfPage, type PdfFont, type PdfTextItem, type PdfStructNode, type PdfLink } from './data.ts'
export { extractPage, readBookmarks, pictureRef, type Extracted, type PdfjsDocument, type PdfjsPage, type PdfjsObjects } from './extract.ts'
export { convertPdfPage, worldOf, PAGE_FRAME_COLOR, type PdfConvertOptions, type PdfPagePlan, type ImageRequest, type EncodedImage, type RasterRequest, type RasterResult } from './convert.ts'
export { pageScale, gridOrigins, stageOf, defaultColumns, PAGE_EDGE, PAGE_GAP } from './layout.ts'
