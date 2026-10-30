/* Table data import (表格数据导入 TODO): pasted HTML / TSV, CSV, xlsx and SQLite → one grid shape → header, field
 * names and types → a TableSource and its records → commit operations. Shared by the Desktop and the Deno CLI host;
 * no DOM, no packages (SQLite is reached through a query function the host hands in). */

export { cell, columnLetter, textGrid, tidy, type Cell, type CellHint, type ColumnHint, type Grid, type GridNote } from './grid.ts'
export { CSV_LIMIT, DELIMITERS, ENCODINGS, decodeText, parseDelimited, readCsv, sniffDelimiter, sniffEncoding, type CsvOptions, type CsvRead, type Delimiter, type Encoding } from './csv.ts'
export { looksLikeTsv, readPastedText, type TextSplit } from './text.ts'
export { looksLikeHtmlTable, readHtmlTable } from './html.ts'
export { openXlsx, sniffWorkbook, XlsxError, type SheetInfo, type Workbook } from './xlsx.ts'
export { affinityHint, isSqlite, listSqliteTables, readSqliteTable, type SqliteQuery, type SqliteTable } from './sqlite.ts'
export { classifyFormat, serialToIso } from './format.ts'
export { ISO_CODES, languageCurrency, resolveAmbiguous, resolveMark, splitMoney } from './currency.ts'
export { IMPORT_TYPES, INVALID, canonicalDecimal, inferColumn, labelsOf, parseDateText, parseNumber, shapeCell, zonedToUtc, type ColumnGuess, type DateOrder, type ImportType, type Json, type ShapeSpec } from './infer.ts'
export { INFER_ROWS, MAX_CELLS, MAX_ROWS, TableLimitError, bodyOf, checkColumn, columnNames, columnsFor, defaultChoices, detectHeader, fieldOf, guessColumns, planTable, retype, specOf, type ColumnCheck, type ColumnChoice, type PlanOptions, type TableChoices, type TablePlan, type Validate } from './plan.ts'
export { RECORDS_PER_OP, ROWS_PER_COMMIT, blockSize, recordIds, tableOperations, uniqueName, type TableCommit, type TableOp, type TableOps, type TableTarget } from './operations.ts'
export { TABULAR_ACCEPT, tabularKind, type RefusedKind, type TabularKind } from './files.ts'
