/* WordprocessingML → a flow page (Office 导入 TODO §3): the body read in order into rich text Blocks, pictures as
 * picture Blocks between them (§12: the rich text schema cannot embed a picture, only a Block, and a flow page shows
 * every Block on it — so a picture is its own Block where the document shows it, and the text around it continues in
 * the next rich text Block).
 *
 *   paragraphs   style chain (`w:pStyle` → `w:basedOn`) by built-in name (Word writes those in English whatever the
 *                style ids are): heading 1–9 / `w:outlineLvl` → heading 1–3, Title → heading 1, Quote → blockquote,
 *                Code* / HTML Preformatted → code block, toc N → dropped; runs of ≥ 2 empty paragraphs → 1
 *   lists        `w:numPr` (direct or by style) and numbering.xml: bullet / none → bullet list, any other format →
 *                ordered with its start (overrides and the count so far, so a list interrupted by a picture goes on
 *                at the right number); levels nested, deeper than the schema allows folded onto the last level
 *   runs         bold / italic / strike(dstrike) → marks (direct and character styles; a paragraph style's only on
 *                plain paragraphs); a character style named Code / Verbatim / HTML Code → code; hidden text dropped;
 *                tab, line break (page and column breaks: nothing), no-break hyphen; hyperlinks (http, https, mailto)
 *   fields       the cached result (HYPERLINK → a link); a TOC field and its entries dropped
 *   revisions    insertions kept, deletions dropped (`keepDeleted`: kept struck through); counted
 *   notes        a reference becomes `[n]`, the notes an ordered list after a rule at the end
 *   tables       cells of inline content (paragraphs joined by line breaks), spans repeated as empty cells, repeated
 *                header rows → header cells; a nested table's text into its cell; pictures in cells after the table
 *   pictures     png / jpeg / gif / webp / svg as they are, bmp for the host to convert; EMF / WMF / TIFF, charts,
 *                SmartArt, OLE objects and formulas through `rasterize` (§5.3) — or dropped (a formula: a
 *                placeholder) and reported; text boxes' paragraphs after the paragraph that anchors them
 *   limits       a rich text Block over the schema's limits is split, at a heading 1 when there is one */

import type { PlanImage } from '../import-drawio/convert.ts'
import { ImportReport } from '../import-drawio/convert.ts'
import { imageSize } from '../import-drawio/bytes.ts'
import { randomId } from '../import-drawio/keys.ts'
import type { XmlElement } from '../import-drawio/xml.ts'
import { attr, coreTitle, descendants, kid, kidPath, kids, local, mediaTypeOf, WEB_IMAGE, type OfficePackage, type Relationship } from './package.ts'
import { extensionOf, fileBaseOf, type DocPlan, type DocSegment, type Rasterize, type RasterRequest } from './plan.ts'
import { inlineText, LIMITS, MAX_HEADING, measure, text, tidyInline, type AstNode, type Mark } from './richtext.ts'
import { docxSubdoc } from './subdoc.ts'
import { emuToPt, int, onOff } from './units.ts'

export interface DocxOptions {
  fileName: string
  /** Unique within the import: plan image keys start with it. */
  idPrefix: string
  /** Keep deleted revision text, struck through (Q4's option); by default deletions are dropped. */
  keepDeleted?: boolean
  newId?: (prefix: string) => string
  /** Turns a one-object sub-document into a picture (LibreOffice on the content server); absent: none can be. */
  rasterize?: Rasterize
  /** The host's words for a formula that could not become a picture. */
  formulaText?: () => string
}

interface RunProps { b?: boolean; i?: boolean; strike?: boolean; vanish?: boolean }
interface StyleInfo { id: string; name: string; type: string; basedOn: string | null; outline: number | null; numId: string | null; ilvl: number | null; run: RunProps; pPr: XmlElement | undefined }

const MONO_CHAR_STYLE = /code|verbatim|html (typewriter|keyboard|sample)/i
const CODE_PARA_STYLE = /code|html preformatted/i
const SYMBOLS: Record<string, string> = { F0B7: '•', F0A7: '▪', F0D8: '➢', F0FC: '✓', F0E0: '→', F06C: '●', F06E: '■', F0A8: '□', F076: '❖' }

function runPropsOf(rPr: XmlElement | undefined): RunProps {
  const out: RunProps = {}
  if (!rPr) return out
  const flag = (name: string) => { const e = kid(rPr, name); return e ? onOff(attr(e, 'val'), true) : undefined }
  const b = flag('b'), i = flag('i'), strike = flag('strike'), dstrike = flag('dstrike'), vanish = flag('vanish')
  if (b !== undefined) out.b = b
  if (i !== undefined) out.i = i
  if (strike !== undefined || dstrike !== undefined) out.strike = Boolean(strike || dstrike)
  if (vanish !== undefined) out.vanish = vanish
  return out
}

class Styles {
  readonly byId = new Map<string, StyleInfo>()
  defaultParagraph: string | null = null
  constructor(doc: XmlElement | null) {
    for (const s of doc ? kids(doc, 'style') : []) {
      const id = attr(s, 'styleId') ?? ''
      const pPr = kid(s, 'pPr')
      const numPr = kid(pPr, 'numPr')
      const outline = kid(pPr, 'outlineLvl')
      const info: StyleInfo = {
        id, name: (attr(kid(s, 'name'), 'val') ?? id).toLowerCase(), type: attr(s, 'type') ?? 'paragraph', basedOn: attr(kid(s, 'basedOn'), 'val') ?? null,
        outline: outline ? int(attr(outline, 'val'), 9) : null, numId: numPr ? attr(kid(numPr, 'numId'), 'val') ?? null : null, ilvl: numPr && kid(numPr, 'ilvl') ? int(attr(kid(numPr, 'ilvl'), 'val')) : null,
        run: runPropsOf(kid(s, 'rPr')), pPr,
      }
      this.byId.set(id, info)
      if (info.type === 'paragraph' && onOff(attr(s, 'default'), attr(s, 'default') !== undefined)) this.defaultParagraph = id
    }
  }
  /** The style and the ones it is based on, nearest first. */
  chain(id: string | null | undefined): StyleInfo[] {
    const out: StyleInfo[] = []
    let at = id ?? null
    while (at && out.length < 16) {
      const s = this.byId.get(at)
      if (!s || out.includes(s)) break
      out.push(s)
      at = s.basedOn
    }
    return out
  }
}

interface NumLevel { format: string; start: number }
class Numbering {
  private readonly abstracts = new Map<string, Map<number, NumLevel>>()
  private readonly nums = new Map<string, { abstract: string; overrides: Map<number, Partial<NumLevel>> }>()
  private readonly links = new Map<string, string>()
  private readonly styles: Styles
  constructor(doc: XmlElement | null, styles: Styles) {
    this.styles = styles
    for (const a of doc ? kids(doc, 'abstractNum') : []) {
      const levels = new Map<number, NumLevel>()
      for (const l of kids(a, 'lvl')) levels.set(int(attr(l, 'ilvl')), { format: attr(kid(l, 'numFmt'), 'val') ?? 'decimal', start: int(attr(kid(l, 'start'), 'val'), 1) })
      const id = attr(a, 'abstractNumId') ?? ''
      this.abstracts.set(id, levels)
      const link = attr(kid(a, 'numStyleLink'), 'val')
      if (link) this.links.set(id, link)
    }
    for (const n of doc ? kids(doc, 'num') : []) {
      const overrides = new Map<number, Partial<NumLevel>>()
      for (const o of kids(n, 'lvlOverride')) {
        const lvl = kid(o, 'lvl')
        const start = kid(o, 'startOverride') ?? kid(lvl, 'start')
        const format = attr(kid(lvl, 'numFmt'), 'val')
        overrides.set(int(attr(o, 'ilvl')), { ...(start ? { start: int(attr(start, 'val'), 1) } : {}), ...(format ? { format } : {}) })
      }
      this.nums.set(attr(n, 'numId') ?? '', { abstract: attr(kid(n, 'abstractNumId'), 'val') ?? '', overrides })
    }
  }
  level(numId: string, ilvl: number, depth = 0): NumLevel {
    const num = this.nums.get(numId)
    if (!num) return { format: 'bullet', start: 1 }
    let levels = this.abstracts.get(num.abstract)
    // a list style's abstract numbering points at the style, whose numbering holds the levels
    const link = this.links.get(num.abstract)
    if (link && depth < 3 && (!levels || levels.size === 0)) {
      const styled = this.styles.chain(link).find((s) => s.numId)?.numId
      if (styled && styled !== numId) return { ...this.level(styled, ilvl, depth + 1), ...num.overrides.get(ilvl) }
    }
    levels ??= new Map()
    const base = levels.get(ilvl) ?? { format: 'decimal', start: 1 }
    return { ...base, ...num.overrides.get(ilvl) }
  }
}

/** A list paragraph: `ilvl` counts in its numbering, `depth` is where it shows (a style's level for "List Bullet 2"). */
type ParaKind = { kind: 'paragraph' } | { kind: 'heading'; level: number } | { kind: 'quote' } | { kind: 'code' } | { kind: 'list'; numId: string; ilvl: number; depth: number; ordered: boolean; start: number }

interface Field { instr: string; phase: 'instr' | 'result'; kind: 'toc' | 'link' | 'other'; url: string | null }

interface ListLevel { node: AstNode; ordered: boolean }
interface OpenList { numId: string; /** The depth of its top level. */ base: number; stack: ListLevel[] }

/** Top-level list nesting the schema's depth allows (doc › list › item › paragraph › text is depth 4; each level 2 more). */
const LIST_LEVELS = Math.floor((LIMITS.max_depth - 4) / 2) + 1

type Item = { kind: 'node'; node: AstNode; empty?: boolean } | { kind: 'image'; segment: Extract<DocSegment, { kind: 'image' }> }

class DocxReader {
  readonly report: ImportReport
  readonly images: PlanImage[] = []
  readonly items: Item[] = []
  readonly counts = { paragraphs: 0, headings: 0, lists: 0, tables: 0, images: 0, rasterized: 0 }
  private readonly pkg: OfficePackage
  private readonly options: DocxOptions
  private readonly newId: (prefix: string) => string
  private styles = new Styles(null)
  private numbering = new Numbering(null, this.styles)
  private rels = new Map<string, Relationship>()
  private readonly imageByPath = new Map<string, PlanImage | null>()
  private readonly fields: Field[] = []
  private list: OpenList | null = null
  private quote: AstNode | null = null
  private code: { node: AstNode; lines: string[] } | null = null
  private readonly counters = new Map<string, number[]>()
  private emptyRun = 0
  private readonly notes: { kind: 'footnote' | 'endnote'; id: string }[] = []
  private noteParts: Record<'footnote' | 'endnote', Map<string, XmlElement>> = { footnote: new Map(), endnote: new Map() }
  private imageCount = 0
  private rasterCount = 0

  constructor(pkg: OfficePackage, report: ImportReport, options: DocxOptions) {
    this.pkg = pkg
    this.report = report
    this.options = options
    this.newId = options.newId ?? randomId
  }

  async read(): Promise<void> {
    const main = this.pkg.main
    const doc = await this.pkg.xml(main)
    const body = kid(doc ?? undefined, 'body')
    if (!doc || !body) throw new Error('no document body')
    this.rels = await this.pkg.rels(main)
    const part = async (type: string) => { const r = [...this.rels.values()].find((x) => x.type === type && !x.external); return r ? await this.pkg.xml(r.target) : null }
    this.styles = new Styles(await part('styles'))
    this.numbering = new Numbering(await part('numbering'), this.styles)
    for (const kind of ['footnote', 'endnote'] as const) {
      const notes = await part(`${kind}s`)
      for (const n of notes ? kids(notes, kind) : []) if (!attr(n, 'type') || attr(n, 'type') === 'normal') this.noteParts[kind].set(attr(n, 'id') ?? '', n)
    }
    await this.blocks(body)
    await this.endNotes()
    this.closeAll()
  }

  // ---- block level

  private async blocks(container: XmlElement): Promise<void> {
    for (const child of container.children) {
      if (typeof child === 'string') continue
      switch (local(child.name)) {
        case 'p': await this.paragraph(child); break
        case 'tbl': await this.table(child); break
        case 'sdt': {
          // a table of contents in a content control is dropped with its entries
          const gallery = attr(descendants(kid(child, 'sdtPr') ?? child, 'docPartGallery')[0], 'val') ?? ''
          if (/table of contents/i.test(gallery)) break
          const content = kid(child, 'sdtContent')
          if (content) await this.blocks(content)
          break
        }
        case 'customXml': case 'smartTag': await this.blocks(child); break
        case 'AlternateContent': { const pick = kid(child, 'Choice') ?? kid(child, 'Fallback'); if (pick) await this.blocks(pick); break }
        case 'altChunk': this.report.add('docx_alt_chunk', 'Embedded foreign content (altChunk) not imported'); break
      }
    }
  }

  private styleOf(pPr: XmlElement | undefined): StyleInfo[] {
    return this.styles.chain(attr(kid(pPr, 'pStyle'), 'val') ?? this.styles.defaultParagraph)
  }

  private classify(pPr: XmlElement | undefined, chain: StyleInfo[]): ParaKind | null {
    const names = chain.map((s) => s.name)
    if (names.some((n) => /^toc\s*\d|^toc heading$/.test(n)) || /^toc\d$/i.test(attr(kid(pPr, 'pStyle'), 'val') ?? '')) return null
    const outlineEl = kid(pPr, 'outlineLvl')
    const outline = outlineEl ? int(attr(outlineEl, 'val'), 9) : chain.find((s) => s.outline !== null)?.outline ?? null
    const named = names.map((n) => /^heading\s*([1-9])$/.exec(n)?.[1]).find(Boolean)
    if (names[0] === 'title') return { kind: 'heading', level: 1 }
    const level = named ? Number(named) : outline !== null && outline < 9 ? outline + 1 : null
    if (level !== null) {
      if (level > MAX_HEADING) this.report.add('docx_heading_level', 'Headings below level 3 imported as level 3', `H${level}`)
      return { kind: 'heading', level: Math.min(MAX_HEADING, level) }
    }
    const numPr = kid(pPr, 'numPr')
    const numId = numPr ? attr(kid(numPr, 'numId'), 'val') ?? null : chain.find((s) => s.numId !== null)?.numId ?? null
    if (numId && numId !== '0') {
      const ilvlEl = kid(numPr, 'ilvl')
      const ilvl = ilvlEl ? int(attr(ilvlEl, 'val')) : chain.find((s) => s.ilvl !== null)?.ilvl ?? 0
      const lvl = this.numbering.level(numId, ilvl)
      // "List Bullet 2" / "List Number 3": a level of its own numbering that shows as a deeper level
      const styled = names.map((n) => /^list (?:bullet|number|continue)\s*([2-9])$/.exec(n)?.[1]).find(Boolean)
      return { kind: 'list', numId, ilvl, depth: ilvl === 0 && styled ? Number(styled) - 1 : ilvl, ordered: lvl.format !== 'bullet' && lvl.format !== 'none', start: lvl.start }
    }
    if (names.some((n) => n === 'quote' || n === 'intense quote')) return { kind: 'quote' }
    if (names.some((n) => CODE_PARA_STYLE.test(n))) return { kind: 'code' }
    return { kind: 'paragraph' }
  }

  private async paragraph(p: XmlElement): Promise<void> {
    const pPr = kid(p, 'pPr')
    const chain = this.styleOf(pPr)
    const kind = this.classify(pPr, chain)
    // a paragraph style's bold / italic only on plain paragraphs (a heading's own look is the heading)
    const base = kind?.kind === 'paragraph' || kind?.kind === 'list' ? [...chain].reverse().reduce<RunProps>((acc, s) => ({ ...acc, ...s.run }), {}) : {}
    const out: AstNode[] = []
    const after: Item[] = []
    const floating: XmlElement[] = []
    const tocBefore = this.fields.some((f) => f.kind === 'toc')
    await this.inline(p, base, null, out, after, floating)
    const tocAfter = this.fields.some((f) => f.kind === 'toc')
    const inline = tidyInline(out)
    // a table of contents' entries (inside its field) are dropped, and so is what is left of the paragraphs it spans
    if (kind && !(tocBefore && tocAfter) && !((tocBefore || tocAfter) && inline.length === 0)) this.place(kind, inline)
    for (const item of after) this.push(item)
    if (floating.length) {
      this.report.add('docx_floating_text', 'Text boxes imported as paragraphs after the paragraph they float by')
      for (const box of floating) await this.blocks(box)
    }
  }

  // ---- inline content

  private marksOf(props: RunProps, link: string | null, code: boolean): Mark[] {
    if (code) return [{ type: 'code' }]
    const marks: Mark[] = []
    if (props.b) marks.push({ type: 'strong' })
    if (props.i) marks.push({ type: 'em' })
    if (props.strike) marks.push({ type: 'strike' })
    if (link) marks.push({ type: 'link', attrs: { href: link } })
    return marks
  }

  private hidden(): boolean {
    return this.fields.some((f) => f.phase === 'instr' || f.kind === 'toc')
  }

  private fieldLink(): string | null {
    for (let k = this.fields.length - 1; k >= 0; k--) if (this.fields[k].kind === 'link' && this.fields[k].phase === 'result') return this.fields[k].url
    return null
  }

  private async inline(container: XmlElement, base: RunProps, link: string | null, out: AstNode[], after: Item[], floating: XmlElement[], deleted = false): Promise<void> {
    for (const child of container.children) {
      if (typeof child === 'string') continue
      const name = local(child.name)
      switch (name) {
        case 'r': await this.run(child, base, link, out, after, floating, deleted); break
        case 'hyperlink': {
          const target = this.rels.get(attr(child, 'id') ?? '')
          const href = target?.external && /^(https?:|mailto:)/i.test(target.target) ? target.target : null
          await this.inline(child, base, href ?? link, out, after, floating, deleted)
          break
        }
        case 'ins': case 'moveTo': this.report.add('docx_revisions', 'Tracked changes accepted (insertions kept, deletions dropped)'); await this.inline(child, base, link, out, after, floating, deleted); break
        case 'del': case 'moveFrom':
          this.report.add('docx_revisions', 'Tracked changes accepted (insertions kept, deletions dropped)')
          if (this.options.keepDeleted) await this.inline(child, { ...base, strike: true }, link, out, after, floating, true)
          break
        case 'fldSimple': {
          const instr = attr(child, 'instr') ?? ''
          const url = /^\s*HYPERLINK\s+"([^"]+)"/i.exec(instr)?.[1] ?? null
          if (/^\s*TOC\b/i.test(instr)) break
          await this.inline(child, base, url && /^(https?:|mailto:)/i.test(url) ? url : link, out, after, floating, deleted)
          break
        }
        case 'smartTag': case 'customXml': case 'bdo': case 'dir': await this.inline(child, base, link, out, after, floating, deleted); break
        case 'sdt': { const content = kid(child, 'sdtContent'); if (content) await this.inline(content, base, link, out, after, floating, deleted); break }
        case 'oMath': case 'oMathPara': await this.formula(child, out, after); break
        case 'AlternateContent': { const pick = kid(child, 'Choice') ?? kid(child, 'Fallback'); if (pick) await this.inline(pick, base, link, out, after, floating, deleted); break }
      }
    }
  }

  private async run(r: XmlElement, base: RunProps, link: string | null, out: AstNode[], after: Item[], floating: XmlElement[], deleted: boolean): Promise<void> {
    const rPr = kid(r, 'rPr')
    const charStyle = this.styles.chain(attr(kid(rPr, 'rStyle'), 'val'))
    const props: RunProps = { ...base, ...[...charStyle].reverse().reduce<RunProps>((acc, s) => ({ ...acc, ...s.run }), {}), ...runPropsOf(rPr), ...(deleted ? { strike: true } : {}) }
    const code = charStyle.some((s) => MONO_CHAR_STYLE.test(s.name))
    if (kid(rPr, 'del') && !deleted) return
    const emit = (value: string) => {
      if (!value || this.hidden() || props.vanish) return
      out.push(text(value, this.marksOf(props, this.fieldLink() ?? link, code)))
    }
    for (const child of r.children) {
      if (typeof child === 'string') continue
      switch (local(child.name)) {
        case 't': emit(child.children.filter((c): c is string => typeof c === 'string').join('')); break
        case 'delText': if (deleted) emit(child.children.filter((c): c is string => typeof c === 'string').join('')); break
        case 'tab': case 'ptab': emit('\t'); break
        case 'br': {
          const type = attr(child, 'type')
          if ((!type || type === 'textWrapping') && !this.hidden() && !props.vanish) out.push({ type: 'hard_break' })
          break
        }
        case 'cr': if (!this.hidden() && !props.vanish) out.push({ type: 'hard_break' }); break
        case 'noBreakHyphen': emit('‑'); break
        case 'sym': {
          const code = (attr(child, 'char') ?? '').toUpperCase()
          const n = parseInt(code, 16)
          emit(SYMBOLS[code] ?? (Number.isFinite(n) && n < 0xf000 ? String.fromCodePoint(n) : ''))
          break
        }
        case 'ruby': { const base2 = kid(child, 'rubyBase'); if (base2) await this.inline(base2, props, link, out, after, floating, deleted); break }
        case 'fldChar': this.fieldChar(attr(child, 'fldCharType') ?? ''); break
        case 'instrText': { const f = this.fields[this.fields.length - 1]; if (f && f.phase === 'instr') f.instr += child.children.filter((c): c is string => typeof c === 'string').join(''); break }
        case 'footnoteReference': case 'endnoteReference': {
          const kind = local(child.name) === 'footnoteReference' ? 'footnote' : 'endnote'
          const id = attr(child, 'id') ?? ''
          if (!this.noteParts[kind].has(id)) break
          this.notes.push({ kind, id })
          emit(`[${this.notes.length}]`)
          break
        }
        case 'commentReference': this.report.add('docx_comments', 'Comments not imported'); break
        case 'drawing': if (!this.hidden()) await this.drawing(child, after, floating); break
        case 'pict': case 'object': if (!this.hidden()) await this.vml(child, after, floating); break
        case 'AlternateContent': {
          // a drawing with its VML fallback: the DrawingML choice, the fallback when the choice gives nothing; a chart or
          // diagram whose fallback is Word's own picture of it: that picture
          const before = after.length + floating.length
          const choice = kid(child, 'Choice')
          const fallbackPicture = kid(child, 'Fallback') && descendants(kid(child, 'Fallback')!, 'blip').concat(descendants(kid(child, 'Fallback')!, 'imagedata')).length > 0
          if (choice && fallbackPicture && descendants(choice, 'graphicData').some((g) => /chart|diagram/.test(attr(g, 'uri') ?? ''))) {
            await this.run({ ...r, children: kid(child, 'Fallback')!.children }, base, link, out, after, floating, deleted)
            break
          }
          if (choice) await this.run({ ...r, children: choice.children }, base, link, out, after, floating, deleted)
          const fallback = kid(child, 'Fallback')
          if (after.length + floating.length === before && fallback) await this.run({ ...r, children: fallback.children }, base, link, out, after, floating, deleted)
          break
        }
      }
    }
  }

  private fieldChar(type: string) {
    if (type === 'begin') { this.fields.push({ instr: '', phase: 'instr', kind: 'other', url: null }); return }
    const f = this.fields[this.fields.length - 1]
    if (!f) return
    if (type === 'separate') {
      f.phase = 'result'
      if (/^\s*TOC\b/i.test(f.instr)) f.kind = 'toc'
      const url = /^\s*HYPERLINK\s+"([^"]+)"/i.exec(f.instr)?.[1]
      if (url && /^(https?:|mailto:)/i.test(url)) { f.kind = 'link'; f.url = url }
    } else if (type === 'end') this.fields.pop()
  }

  // ---- pictures

  private async imageFromPart(path: string): Promise<{ image: PlanImage | null; reason: string | null }> {
    if (this.imageByPath.has(path)) return { image: this.imageByPath.get(path) ?? null, reason: null }
    const bytes = await this.pkg.bytes(path)
    const mediaType = mediaTypeOf(this.pkg, path)
    let image: PlanImage | null = null
    let reason: string | null = null
    if (!bytes) reason = 'missing'
    else if (WEB_IMAGE.has(mediaType) || mediaType === 'image/bmp') {
      this.imageCount += 1
      image = { key: `${this.options.idPrefix}i${this.imageCount}`, name: `${fileBaseOf(this.options.fileName)}-img${this.imageCount}.${extensionOf(mediaType)}`, mediaType, bytes }
    } else reason = mediaType
    this.imageByPath.set(path, image)
    if (image) this.images.push(image)
    return { image, reason }
  }

  private async rasterImage(kind: RasterRequest['kind'], object: XmlElement, size: { w: number; h: number } | null, name: string, trim: boolean): Promise<PlanImage | null> {
    if (!this.options.rasterize) return null
    const w = size?.w ?? 451, h = size?.h ?? 120
    let subdoc: Uint8Array
    try { subdoc = await docxSubdoc(this.pkg, object, w, h) } catch { return null }
    const got = await this.options.rasterize({ kind, subdoc, subdocKind: 'docx', width: w, height: h, name, trim }).catch(() => null)
    if (!got) return null
    this.rasterCount += 1
    this.counts.rasterized += 1
    const image: PlanImage = { key: `${this.options.idPrefix}r${this.rasterCount}`, name: `${fileBaseOf(this.options.fileName)}-${kind}${this.rasterCount}.${extensionOf(got.mediaType)}`, mediaType: got.mediaType, bytes: got.bytes }
    this.images.push(image)
    this.report.add('docx_rasterized', 'Charts, diagrams, formulas and pictures in other formats imported as pictures (LibreOffice)', name)
    return image
  }

  private pictureItem(image: PlanImage, size: { w: number; h: number } | null, title: string | undefined): Item {
    this.counts.images += 1
    const px = imageSize(image.bytes ?? new Uint8Array(0), image.mediaType)
    const s = size ?? (px ? { w: px.width * 0.75, h: px.height * 0.75 } : null)
    const pixels = px ? { width: Math.round(px.width), height: Math.round(px.height) } : null
    return { kind: 'image', segment: { kind: 'image', image: image.key, size: s, pixels, ...(title ? { title: title.slice(0, 256) } : {}) } }
  }

  /** A picture by relationship id: as it is, or rasterized (EMF, WMF, TIFF …), or reported. */
  private async picture(relId: string, object: XmlElement, size: { w: number; h: number } | null, title: string | undefined, after: Item[]) {
    const rel = this.rels.get(relId)
    if (!rel) return
    if (rel.external) { this.report.add('docx_external_image', 'Linked (not embedded) pictures not imported', rel.target.slice(0, 80)); return }
    const { image, reason } = await this.imageFromPart(rel.target)
    if (image) { after.push(this.pictureItem(image, size, title)); return }
    const raster = reason && reason !== 'missing' ? await this.rasterImage('image', object, size, rel.target.replace(/.*\//, ''), false) : null
    if (raster) after.push(this.pictureItem(raster, size, title))
    else this.report.add('docx_unsupported_image', 'Pictures in formats a browser cannot show (EMF, WMF, TIFF) not imported', rel.target.replace(/.*\//, ''))
  }

  private async drawing(drawing: XmlElement, after: Item[], floating: XmlElement[]) {
    const holder = kid(drawing, 'inline') ?? kid(drawing, 'anchor')
    if (!holder) return
    const extent = kid(holder, 'extent')
    const size = extent ? { w: emuToPt(int(attr(extent, 'cx'))), h: emuToPt(int(attr(extent, 'cy'))) } : null
    const docPr = kid(holder, 'docPr')
    const title = attr(docPr, 'descr') || attr(docPr, 'title') || undefined
    const data = kidPath(holder, 'graphic', 'graphicData')
    const uri = attr(data, 'uri') ?? ''
    if (!data) return
    if (/\/picture$/.test(uri)) {
      const blip = descendants(data, 'blip')[0]
      const id = attr(blip, 'embed') ?? attr(blip, 'link')
      if (id) await this.picture(id, drawing, size, title, after)
      return
    }
    const rasterKind: RasterRequest['kind'] | null = /chart/.test(uri) ? 'chart' : /diagram/.test(uri) ? 'smartart' : null
    if (rasterKind) {
      const raster = await this.rasterImage(rasterKind, drawing, size, title ?? attr(docPr, 'name') ?? rasterKind, false)
      if (raster) after.push(this.pictureItem(raster, size, title))
      else this.report.add(`docx_${rasterKind}`, rasterKind === 'chart' ? 'Charts not imported' : 'SmartArt diagrams not imported', attr(docPr, 'name'))
      return
    }
    // shapes, groups and canvases: their pictures and text boxes
    const boxes = descendants(data, 'txbxContent')
    floating.push(...boxes)
    const pics = descendants(data, 'pic').filter((p) => kid(p, 'blipFill'))
    for (const pic of pics) {
      const blip = descendants(pic, 'blip')[0]
      const id = attr(blip, 'embed')
      const ext = descendants(pic, 'ext').find((e) => attr(e, 'cx'))
      if (id) await this.picture(id, drawing, ext ? { w: emuToPt(int(attr(ext, 'cx'))), h: emuToPt(int(attr(ext, 'cy'))) } : size, title, after)
    }
    if (!boxes.length && !pics.length) this.report.add('docx_shapes_dropped', 'Drawn shapes not imported', attr(docPr, 'name'))
  }

  /** VML: `w:pict` (a picture or a text box) and `w:object` (an OLE object's preview picture). */
  private async vml(el: XmlElement, after: Item[], floating: XmlElement[]) {
    const boxes = descendants(el, 'txbxContent')
    floating.push(...boxes)
    const imageData = descendants(el, 'imagedata')[0]
    const id = attr(imageData, 'id') ?? attr(imageData, 'pict')
    const shape = descendants(el, 'shape')[0]
    const style = shape?.attrs.style ?? ''
    const len = (prop: string) => { const m = new RegExp(`${prop}:\\s*([\\d.]+)(pt|in|px)?`).exec(style); if (!m) return null; const v = Number(m[1]); return m[2] === 'in' ? v * 72 : m[2] === 'px' ? v * 0.75 : v }
    const w = len('width'), h = len('height')
    const size = w && h ? { w, h } : null
    if (local(el.name) === 'object') this.report.add('docx_ole', 'Embedded objects (OLE) imported as their preview pictures')
    if (id) await this.picture(id, el, size, undefined, after)
    else if (!boxes.length && local(el.name) === 'object') {
      const raster = await this.rasterImage('ole', el, size, 'object', false)
      if (raster) after.push(this.pictureItem(raster, size, undefined))
    }
  }

  private async formula(math: XmlElement, out: AstNode[], after: Item[]) {
    const raster = await this.rasterImage('formula', math, null, 'formula', true)
    if (raster) { after.push(this.pictureItem(raster, null, undefined)); return }
    this.report.add('docx_formula', 'Formulas not imported (a placeholder marks each)')
    out.push(text(this.options.formulaText?.() ?? '[Formula]'))
  }

  // ---- tables

  private async table(tbl: XmlElement) {
    const rows: AstNode[] = []
    const after: Item[] = []
    const rowsOf = (el: XmlElement): XmlElement[] => el.children.flatMap((c) => (typeof c === 'string' ? [] : local(c.name) === 'tr' ? [c] : ['sdt', 'customXml'].includes(local(c.name)) ? rowsOf(kid(c, 'sdtContent') ?? c) : []))
    const cellsOf = (el: XmlElement): XmlElement[] => el.children.flatMap((c) => (typeof c === 'string' ? [] : local(c.name) === 'tc' ? [c] : ['sdt', 'customXml'].includes(local(c.name)) ? cellsOf(kid(c, 'sdtContent') ?? c) : []))
    let width = 0
    for (const tr of rowsOf(tbl)) {
      const trPr = kid(tr, 'trPr')
      const header = Boolean(kid(trPr, 'tblHeader')) && onOff(attr(kid(trPr, 'tblHeader'), 'val'), true)
      const cells: AstNode[] = []
      const cell = (content: AstNode[]): AstNode => ({ type: 'table_cell', ...(header ? { attrs: { header: 1 } } : {}), content })
      for (let k = 0; k < int(attr(kid(trPr, 'gridBefore'), 'val')); k++) cells.push(cell([]))
      for (const tc of cellsOf(tr)) {
        const tcPr = kid(tc, 'tcPr')
        const span = Math.max(1, int(attr(kid(tcPr, 'gridSpan'), 'val'), 1))
        const vMerge = kid(tcPr, 'vMerge')
        const continued = vMerge !== undefined && attr(vMerge, 'val') !== 'restart'
        if (span > 1 || continued) this.report.add('docx_merged_cells', 'Merged table cells split (the text stays in the first)')
        cells.push(cell(continued ? [] : await this.cellContent(tc, after)))
        for (let k = 1; k < span; k++) cells.push(cell([]))
      }
      for (let k = 0; k < int(attr(kid(trPr, 'gridAfter'), 'val')); k++) cells.push(cell([]))
      if (cells.length === 0) continue
      width = Math.max(width, cells.length)
      rows.push({ type: 'table_row', content: cells })
    }
    if (rows.length) {
      for (const row of rows) while ((row.content?.length ?? 0) < width) row.content!.push({ type: 'table_cell', content: [] })
      this.counts.tables += 1
      this.push({ kind: 'node', node: { type: 'table', attrs: { block_id: this.newId('b') }, content: rows } })
    }
    for (const item of after) this.push(item)
  }

  /** A cell's paragraphs as inline content joined by line breaks; a nested table's rows as lines of text. */
  private async cellContent(tc: XmlElement, after: Item[]): Promise<AstNode[]> {
    const lines: AstNode[][] = []
    const walk = async (el: XmlElement) => {
      for (const c of el.children) {
        if (typeof c === 'string') continue
        const name = local(c.name)
        if (name === 'p') {
          const out: AstNode[] = []
          const floating: XmlElement[] = []
          const pPr = kid(c, 'pPr')
          const kind = this.classify(pPr, this.styleOf(pPr))
          await this.inline(c, {}, null, out, after, floating)
          const line = tidyInline(out)
          if (kind?.kind === 'list') line.unshift(text(kind.ordered ? '' : '• '))
          lines.push(line)
          for (const box of floating) await walk(box)
        } else if (name === 'tbl') {
          this.report.add('docx_nested_table', 'Tables inside table cells imported as lines of text')
          const nested: AstNode[][] = []
          for (const tr of descendants(c, 'tr')) {
            const parts: string[] = []
            for (const tc2 of kids(tr, 'tc')) {
              const inner: AstNode[] = []
              for (const p of descendants(tc2, 'p')) await this.inline(p, {}, null, inner, after, [])
              parts.push(inlineText(tidyInline(inner)).replace(/\n/g, ' '))
            }
            nested.push([text(parts.join(' | '))])
          }
          lines.push(...nested)
        } else if (name === 'sdt') { const content = kid(c, 'sdtContent'); if (content) await walk(content) }
        else if (name === 'customXml') await walk(c)
      }
    }
    await walk(tc)
    // empty trailing lines (Word ends every cell with a paragraph) are not content
    while (lines.length && lines[lines.length - 1].length === 0) lines.pop()
    const out: AstNode[] = []
    lines.forEach((line, k) => { if (k > 0) out.push({ type: 'hard_break' }); out.push(...line) })
    return tidyInline(out)
  }

  // ---- notes

  private async endNotes() {
    if (!this.notes.length) return
    const items: AstNode[] = []
    for (const n of this.notes) {
      const part = this.noteParts[n.kind].get(n.id)
      const out: AstNode[] = []
      for (const [k, p] of kids(part, 'p').entries()) {
        if (k > 0) out.push({ type: 'hard_break' })
        const tmp: AstNode[] = []
        await this.inline({ ...p, children: p.children.filter((c) => typeof c === 'string' || !(local(c.name) === 'r' && kids(c, 'footnoteRef').length + kids(c, 'endnoteRef').length > 0)) }, {}, null, tmp, [], [])
        out.push(...tidyInline(tmp))
      }
      const content = tidyInline(out)
      if (content[0]?.type === 'text') content[0] = { ...content[0], text: (content[0].text ?? '').replace(/^\s+/, '') }
      items.push({ type: 'list_item', attrs: { block_id: this.newId('b') }, content: [{ type: 'paragraph', attrs: { block_id: this.newId('b') }, content: tidyInline(content) }] })
    }
    this.closeAll()
    this.push({ kind: 'node', node: { type: 'horizontal_rule', attrs: { block_id: this.newId('b') } } })
    this.push({ kind: 'node', node: { type: 'ordered_list', attrs: { block_id: this.newId('b') }, content: items } })
  }

  // ---- building

  private paragraphNode(type: 'paragraph' | 'heading', content: AstNode[], level?: number): AstNode {
    return { type, attrs: { block_id: this.newId('b'), ...(type === 'heading' ? { level: level ?? 1 } : {}) }, ...(content.length ? { content } : {}) }
  }

  private closeList() { this.list = null }
  private closeQuote() { this.quote = null }
  private closeCode() { this.code = null }
  private closeAll() { this.closeList(); this.closeQuote(); this.closeCode() }

  /** A table, a picture or the notes: whatever list, quote or code block was open ends. */
  private push(item: Item) {
    this.closeAll()
    this.emptyRun = 0
    this.items.push(item)
  }

  private place(kind: ParaKind, inline: AstNode[]) {
    const empty = inline.length === 0
    if (kind.kind !== 'list') this.closeList()
    if (kind.kind !== 'quote') this.closeQuote()
    if (kind.kind !== 'code') this.closeCode()
    switch (kind.kind) {
      case 'heading':
        this.emptyRun = 0
        this.counts.headings += 1
        this.items.push({ kind: 'node', node: this.paragraphNode('heading', inline, kind.level) })
        return
      case 'quote': {
        if (empty) { this.closeQuote(); return }
        this.emptyRun = 0
        if (!this.quote) { this.quote = { type: 'blockquote', attrs: { block_id: this.newId('b') }, content: [] }; this.items.push({ kind: 'node', node: this.quote }) }
        this.quote.content!.push(this.paragraphNode('paragraph', inline))
        return
      }
      case 'code': {
        this.emptyRun = 0
        if (!this.code) { this.code = { node: { type: 'code_block', attrs: { block_id: this.newId('b') } }, lines: [] }; this.items.push({ kind: 'node', node: this.code.node }) }
        this.code.lines.push(inlineText(inline))
        const value = this.code.lines.join('\n')
        if (value) this.code.node.content = [text(value)]
        return
      }
      case 'list': this.emptyRun = 0; this.listItem(kind, inline); return
      case 'paragraph':
        this.counts.paragraphs += empty ? 0 : 1
        if (empty) {
          this.emptyRun += 1
          if (this.emptyRun > 1) return
        } else this.emptyRun = 0
        this.items.push({ kind: 'node', node: this.paragraphNode('paragraph', inline), empty })
    }
  }

  private listItem(kind: Extract<ParaKind, { kind: 'list' }>, inline: AstNode[]) {
    // Word numbering: a level counts on, the levels below it start again
    const counts = this.counters.get(kind.numId) ?? []
    counts[kind.ilvl] = (counts[kind.ilvl] ?? kind.start - 1) + 1
    counts.length = kind.ilvl + 1
    this.counters.set(kind.numId, counts)
    // another numbering at the top starts a list of its own (a restart, a new list); deeper, it nests in the open one
    if (this.list && this.list.numId !== kind.numId && kind.depth <= this.list.base) this.closeList()
    const newList = (ordered: boolean, start: number): AstNode => ({ type: ordered ? 'ordered_list' : 'bullet_list', attrs: { block_id: this.newId('b'), ...(ordered && start > 1 ? { start } : {}) }, content: [] })
    const item = (): AstNode => ({ type: 'list_item', attrs: { block_id: this.newId('b') }, content: [this.paragraphNode('paragraph', inline)] })
    if (!this.list) {
      const root = newList(kind.ordered, counts[kind.ilvl])
      this.list = { numId: kind.numId, base: kind.depth, stack: [{ node: root, ordered: kind.ordered }] }
      this.counts.lists += 1
      this.items.push({ kind: 'node', node: root })
      root.content!.push(item())
      return
    }
    let level = Math.max(0, kind.depth - this.list.base)
    if (level >= LIST_LEVELS) { this.report.add('docx_list_depth', 'List levels deeper than three imported at the third'); level = LIST_LEVELS - 1 }
    const stack = this.list.stack
    while (stack.length - 1 > level) stack.pop()
    while (stack.length - 1 < level) {
      const parent = stack[stack.length - 1].node
      const last = parent.content![parent.content!.length - 1]
      const nested = newList(kind.ordered, counts[kind.ilvl])
      last.content!.push(nested)
      stack.push({ node: nested, ordered: kind.ordered })
    }
    stack[stack.length - 1].node.content!.push(item())
  }
}

/** Rich text Blocks under the schema's limits: top-level nodes in order, a new Block before a heading 1 once the
 * current one is half full, and wherever the next node would not fit. */
export function splitDoc(nodes: AstNode[], limits: { blocks: number; text: number } = { blocks: LIMITS.max_blocks, text: LIMITS.max_text_utf16 }): AstNode[][] {
  const out: AstNode[][] = []
  let current: AstNode[] = []
  let blocks = 0, length = 0
  for (const node of nodes) {
    const m = measure(node)
    const h1 = node.type === 'heading' && (node.attrs?.level ?? 1) === 1
    const full = blocks + m.blocks > limits.blocks || length + m.text > limits.text
    const half = blocks > limits.blocks / 2 || length > limits.text / 2
    if (current.length && (full || (h1 && half))) { out.push(current); current = []; blocks = 0; length = 0 }
    current.push(node)
    blocks += m.blocks
    length += m.text
  }
  if (current.length) out.push(current)
  return out
}

/** Convert a docx (an opened package) into a flow page plan. */
export async function convertDocx(pkg: OfficePackage, report: ImportReport, options: DocxOptions): Promise<DocPlan> {
  const reader = new DocxReader(pkg, report, options)
  await reader.read()
  const segments: DocSegment[] = []
  let run: Item[] = []
  const flush = () => {
    // empty paragraphs at either end of a Block are only spacing
    while (run.length && run[0].kind === 'node' && run[0].empty) run.shift()
    while (run.length && run[run.length - 1].kind === 'node' && (run[run.length - 1] as { empty?: boolean }).empty) run.pop()
    const nodes = run.map((i) => (i.kind === 'node' ? i.node : null)).filter((n): n is AstNode => n !== null)
    run = []
    if (!nodes.length) return
    const parts = splitDoc(nodes)
    if (parts.length > 1) report.add('docx_split', 'Long documents split into several rich text Blocks', String(parts.length))
    for (const part of parts) segments.push({ kind: 'richtext', content: { type: 'doc', content: part } })
  }
  for (const item of reader.items) {
    if (item.kind === 'image') { flush(); segments.push(item.segment) } else run.push(item)
  }
  flush()
  const title = (await coreTitle(pkg)) || options.fileName.replace(/\.[^.]+$/, '')
  return { title: title.slice(0, 120), segments, images: reader.images, counts: reader.counts }
}
