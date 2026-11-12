/* Block-controls shared by the Desktop canvas (alias `@controls/*`), Deno (the aiws program host, the catalogue build,
 * tests) and Agents: the `tree` layout, node measuring, implicit edges, Markdown outline / OPML conversion and the
 * control catalogue. No DOM, no dependencies (脑图 TODO M1). */

export { CONTROL_KINDS, COMPOSITE, DATA_CONTROLS, TEXT_VIEWS, isControlKind, isDataControl, isTextView, SLOT_PATTERN } from './contract.ts'
export { AUTO, ALL_AUTO, hasAuto, isAuto, isPinned, num, type Axis, type StoredPlacement } from './placement.ts'
export { parseInline, plainText, formatInline, type Run, type RunStyle } from './inline.ts'
export { NodeMeasurer, approxMetrics, typography, runFont, wrapRuns, NODE_PAD, MAX_TEXT_WIDTH, type TextMetrics, type MeasuredNode, type MeasuredLine } from './measure.ts'
export { layoutTree, mindmapConfig, DEFAULT_CONFIG, BOX_PADDING, allIds, type Direction, type EdgeStyle, type Grow, type MindmapConfig, type TreeNode, type LaidNode, type TreeLayout, type Rect } from './tree.ts'
export { edgePath, edgeEnds, edgeStrokes, branchColors, edgeWidth, BRANCH_PALETTE, type EdgeStroke } from './edges.ts'
export { solveControl, solveAny, solveBox, placeControl, type ControlEntity, type ControlSource, type SolveOptions, type SolvedControl, type PlacedNode, type PlacedControl } from './solve.ts'
export { parseMarkdownOutline, formatMarkdownOutline, nodeOperations, mindmapOperations, controlOutline, readerOf, nodeTitle, MAX_TITLE_CHARS, type OutlineNode, type MindmapCreate, type TreeReader, type Op } from './outline.ts'
export { parseOpml, formatOpml, OpmlError } from './opml.ts'
export { solveComposite, compositeOf, templated, dimension, paddingOf, stackedSlots, stackedRegions, projectionLayout, type CompositionProjection, type CompositionMedia, showsTitle, slotAccepts, slotFor, TITLE_BAND, EMPTY_BOX, DEFAULT_SPACING, DEFAULT_PADDING, type Composition, type CompositionSlot, type SlotPlacement } from './composite.ts'
export { CONTROLS, controlDescriptor, controlsCatalog, catalogJson, type ControlDescriptor } from './catalog.ts'
export { projectedId, parseProjectedId, resolveValue, holds, nodeRect, instantiate, project, type ProjectedView, type Dim, type TemplateBox, type TemplateNode, type Template, type ProjectedNode, type Scope } from './projection.ts'
export { layoutList, insertionAt, layoutColumns, columnDropAt, layoutMonth, monthCell, monthDayAt, layoutTimeGrid, minuteY, timeAt, layoutAgenda, type ListItem, type ColumnSpec, type ColumnItem, type LaidColumn, type ColumnsOptions, type MonthEvent, type MonthOptions, type MonthBar, type MonthMore, type TimedPiece, type TimeGridOptions, type AgendaGroup } from './datalayout.ts'
export { TODO_CONTROL, CALENDAR_CONTROL } from './schedule.ts'
