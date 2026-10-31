/* The shared draw.io module's names and messages in the Desktop's language. The module (no DOM, no Desktop code)
 * hands stable ids and codes — compass anchor ids, import report kinds, file format error codes, library ids — with
 * English text for developers only; these are the words the canvas shows for them. (Shape kind and group names are
 * the shape Block's, in ui/blocks/shapeBlock.tsx.) */

import type { AnchorDef } from '@drawio/anchors'
import type { ReportNote } from '@drawio/convert'
import { DrawioFormatError, type DrawioFormatCode } from '@drawio/file'
import { aiwsT, type TextVars } from '../../i18n'

type Translate = (key: string, vars?: TextVars) => string

const own = (table: Record<string, string>, id: string): string | undefined => (Object.hasOwn(table, id) ? table[id] : undefined)

const COMPASS: Record<string, string> = {
  n: 'aiworkspace.drawio.anchor.n', nne: 'aiworkspace.drawio.anchor.nne', ne: 'aiworkspace.drawio.anchor.ne', ene: 'aiworkspace.drawio.anchor.ene',
  e: 'aiworkspace.drawio.anchor.e', ese: 'aiworkspace.drawio.anchor.ese', se: 'aiworkspace.drawio.anchor.se', sse: 'aiworkspace.drawio.anchor.sse',
  s: 'aiworkspace.drawio.anchor.s', ssw: 'aiworkspace.drawio.anchor.ssw', sw: 'aiworkspace.drawio.anchor.sw', wsw: 'aiworkspace.drawio.anchor.wsw',
  w: 'aiworkspace.drawio.anchor.w', wnw: 'aiworkspace.drawio.anchor.wnw', nw: 'aiworkspace.drawio.anchor.nw', nnw: 'aiworkspace.drawio.anchor.nnw',
}

/** An anchor's name in tooltips: its own label, else the compass name of its id, else the id. */
export function anchorName(anchor: AnchorDef | undefined, id: string, t: Translate = aiwsT): string {
  if (anchor?.label) return anchor.label
  const key = own(COMPASS, id)
  return key ? t(key) : id
}

const LIBRARIES: Record<string, string> = {
  flowchart: 'aiworkspace.drawio.library.flowchart', basic: 'aiworkspace.drawio.library.basic', arrows: 'aiworkspace.drawio.library.arrows',
  bpmn: 'aiworkspace.drawio.library.bpmn', networks: 'aiworkspace.drawio.library.networks',
}

/** A shape library's name by its id (the packs' own `title` is the build script's, for an unknown library). */
export function libraryTitle(lib: string, title: string, t: Translate = aiwsT): string {
  const key = own(LIBRARIES, lib)
  return key ? t(key) : title
}

const REPORT: Record<string, string> = {
  complex_html: 'aiworkspace.drawio.report.complexHtml', rich_label: 'aiworkspace.drawio.report.richLabel', gradient: 'aiworkspace.drawio.report.gradient',
  shadow: 'aiworkspace.drawio.report.shadow', sketch: 'aiworkspace.drawio.report.sketch', glass: 'aiworkspace.drawio.report.glass',
  partial_border: 'aiworkspace.drawio.report.partialBorder', collapsed: 'aiworkspace.drawio.report.collapsed', table: 'aiworkspace.drawio.report.table',
  group_rotation: 'aiworkspace.drawio.report.groupRotation', image_missing: 'aiworkspace.drawio.report.imageMissing', svg_too_large: 'aiworkspace.drawio.report.svgTooLarge',
  approximated: 'aiworkspace.drawio.report.approximated', stencil_failed: 'aiworkspace.drawio.report.stencilFailed', library_shape: 'aiworkspace.drawio.report.libraryShape',
  unknown_shape: 'aiworkspace.drawio.report.unknownShape', hidden_cell: 'aiworkspace.drawio.report.hiddenCell', cap: 'aiworkspace.drawio.report.cap',
  anchor_on_box: 'aiworkspace.drawio.report.anchorOnBox', edge_to_edge: 'aiworkspace.drawio.report.edgeToEdge', cap_size: 'aiworkspace.drawio.report.capSize',
  jump: 'aiworkspace.drawio.report.jump', flow_animation: 'aiworkspace.drawio.report.flowAnimation', extra_labels: 'aiworkspace.drawio.report.extraLabels',
  hidden_layer: 'aiworkspace.drawio.report.hiddenLayer',
  // PDF import (PDF 导入方案 §5): the converter's codes
  pdf_image_sheared: 'aiworkspace.pdf.report.imageSheared', pdf_image_alpha: 'aiworkspace.pdf.report.imageAlpha', pdf_image_clipped: 'aiworkspace.pdf.report.imageClipped',
  pdf_image_unreadable: 'aiworkspace.pdf.report.imageUnreadable', pdf_unreadable_text: 'aiworkspace.pdf.report.unreadableText', pdf_mixed_style: 'aiworkspace.pdf.report.mixedStyle',
  pdf_invisible_text: 'aiworkspace.pdf.report.invisibleText', pdf_vertical_text: 'aiworkspace.pdf.report.verticalText', pdf_complex_page: 'aiworkspace.pdf.report.complexPage',
  pdf_complex_whole: 'aiworkspace.pdf.report.complexWhole', pdf_effects: 'aiworkspace.pdf.report.effects', pdf_svg_too_large: 'aiworkspace.pdf.report.svgTooLarge',
  pdf_raster_parts: 'aiworkspace.pdf.report.rasterParts', pdf_raster_failed: 'aiworkspace.pdf.report.rasterFailed', pdf_page_failed: 'aiworkspace.pdf.report.pageFailed',
  pdf_text_layout_limit: 'aiworkspace.pdf.report.textLayoutLimit',
  pdf_original_missing: 'aiworkspace.pdf.report.originalMissing', pdf_path_failed: 'aiworkspace.pdf.report.pathFailed',
  // table import (表格数据导入 TODO §7): the importer's codes
  table_empty_rows: 'aiworkspace.import.report.emptyRows', table_merged_header: 'aiworkspace.import.report.mergedHeader', table_merged_cells: 'aiworkspace.import.report.mergedCells',
  table_formula: 'aiworkspace.import.report.formula', table_error_cells: 'aiworkspace.import.report.errorCells', table_time_only: 'aiworkspace.import.report.timeOnly',
  table_blob_column: 'aiworkspace.import.report.blobColumn', table_rows_limit: 'aiworkspace.import.report.rowsLimit', table_as_text: 'aiworkspace.import.report.asText',
  table_percent: 'aiworkspace.import.report.percent', table_precision: 'aiworkspace.import.report.precision', table_long_text: 'aiworkspace.import.report.longText',
  table_invalid: 'aiworkspace.import.report.invalid', table_currency_symbol: 'aiworkspace.import.report.currencySymbol', table_keep_failed: 'aiworkspace.import.report.keepFailed',
}

/** What an import report note says, by its kind (`stencil:<instruction>` names the instruction). */
export function reportNoteText(note: ReportNote, t: Translate = aiwsT): string {
  if (note.kind.startsWith('stencil:')) return t('aiworkspace.drawio.report.stencilInstruction', { instruction: note.kind.slice('stencil:'.length) })
  const key = own(REPORT, note.kind)
  return key ? t(key) : note.message
}

const FORMAT_ERRORS: Record<DrawioFormatCode, string> = {
  no_pages: 'aiworkspace.drawio.error.noPages', library_not_json: 'aiworkspace.drawio.error.libraryNotJson', svg_without_diagram: 'aiworkspace.drawio.error.svgWithoutDiagram',
  unknown_root: 'aiworkspace.drawio.error.unknownRoot', png_without_diagram: 'aiworkspace.drawio.error.pngWithoutDiagram', empty: 'aiworkspace.drawio.error.empty',
}

/** Why a file is not a readable draw.io file, when the module said so (a DrawioFormatError), else null. */
export function formatErrorText(error: unknown, t: Translate = aiwsT): string | null {
  if (!(error instanceof DrawioFormatError)) return null
  return t(FORMAT_ERRORS[error.code], { name: error.detail ?? '' })
}

/** The text of the dashed box that stands in for a picture the import cannot have. */
export function imageFallbackText(url: string | null, t: Translate = aiwsT): string {
  return url ? t('aiworkspace.drawio.imageFallbackUrl', { url }) : t('aiworkspace.drawio.imageFallback')
}
