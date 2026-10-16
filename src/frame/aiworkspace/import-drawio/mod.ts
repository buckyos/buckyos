/* draw.io import (draw.io 导入 TODO §3): read a file, convert its pages into import plans, turn a plan into
 * commit operations. Shared by the Desktop canvas and the aiws program host (Deno); no DOM, no dependencies. */

export { readDrawio, looksLikeDrawio, DrawioFormatError, type DrawioFormatCode, type DrawioFile, type DrawioPage, type LibraryEntry } from './file.ts'
export { convertPage, ImportReport, type ConvertOptions, type PagePlan, type PlanItem, type PlanImage, type ReportNote, type ResolvedStencil, type Json } from './convert.ts'
export { buildOperations, batches, type BuildTarget, type BuiltPage, type ImportOp, type UploadedAsset } from './operations.ts'
export { exportDiagram, type ExportNode, type ExportPage, type ExportResult } from './export.ts'
