/* Office document import (Office 导入 TODO): .docx → a flow page (rich text and picture Blocks), .pptx → canvas pages
 * like the PDF importer's. Shared by the Desktop and Deno (the Agent command); no DOM, no packages. */

export { EDITABLE_KINDS, OFFICE_ACCEPT, OfficeError, coreTitle, isDocumentKind, mediaTypeOf, officeKind, openPackage, type OfficeKind, type OfficePackage, type Relationship } from './package.ts'
export { DEFAULT_THEME, colorIn, colorOf, readClrMap, readTheme, themeFont, type ColorContext, type Rgba, type Theme } from './theme.ts'
export { LIMITS, MAX_HEADING, measure, validateDoc, type AstNode, type Mark } from './richtext.ts'
export { extensionOf, fileBaseOf, type DocPlan, type DocSegment, type EncodedImage, type OfficePagePlan, type Rasterize, type RasterRequest } from './plan.ts'
export { convertDocx, splitDoc, type DocxOptions } from './docx.ts'
export { buildDocOperations, sizedBatches, type BuiltDoc, type DocTarget } from './operations.ts'
export { crc32, docxSubdoc, pptxSubdoc, writeZip } from './subdoc.ts'
export { PAGE_FRAME_COLOR, SLIDE_OBJECT_LIMIT, convertSlide, readDeck, type CropRequest, type Deck, type DeckSlide, type PptxOptions } from './pptx.ts'
export { adjustOf, customDef, evaluate, presetNames, presetPaths, type GeomPath, type PresetDef } from './geometry.ts'
export { autoNumber, readParagraphs, shapeText } from './text.ts'
