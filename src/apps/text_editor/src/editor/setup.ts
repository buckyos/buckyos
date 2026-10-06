import { Annotation, Compartment, EditorState, Transaction, type Extension } from '@codemirror/state'
import { EditorView, keymap, lineNumbers, highlightActiveLine, drawSelection, rectangularSelection, highlightWhitespace } from '@codemirror/view'
import { defaultKeymap, history, historyKeymap, indentWithTab, undo, redo } from '@codemirror/commands'
import { selectNextOccurrence, search, searchKeymap, openSearchPanel, gotoLine } from '@codemirror/search'
import { bracketMatching, defaultHighlightStyle, foldGutter, indentOnInput, indentUnit, syntaxHighlighting } from '@codemirror/language'
import { closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete'
import { languageFor } from './languages.ts'
import type { EditorDocument, Workspace } from '../model/workspace.ts'
const sync = Annotation.define<boolean>()
export class DocumentEditor {
  state: EditorState
  views = new Set<EditorView>()
  private config = new Compartment()
  private language = new Compartment()
  private languageKey = ''
  private configKey = ''
  private serial = 0
  doc: EditorDocument; workspace: Workspace
  constructor(doc: EditorDocument, workspace: Workspace) {
    this.doc = doc; this.workspace = workspace
    this.state = EditorState.create({ doc: doc.text, extensions: [history(), EditorState.transactionFilter.of(tr => tr.docChanged && (doc.readOnly || doc.mutating) && !tr.annotation(sync) ? [] : tr), this.config.of([]), this.language.of([])] })
    this.configure()
  }
  private get primary(): EditorView | undefined { return [...this.views][0] }
  attach(parent: HTMLElement, onUpdate: (view: EditorView) => void): EditorView {
    const view = new EditorView({ state: this.state, parent, dispatch: transaction => {
      this.dispatch(view, transaction); onUpdate(view)
    } })
    this.views.add(view); return view
  }
  detach(view: EditorView): void { this.state = this.primary?.state ?? view.state; this.views.delete(view); view.destroy() }
  dispatch(view: EditorView, transaction: Transaction): void {
    view.update([transaction])
    if (transaction.docChanged && !transaction.annotation(sync)) {
      for (const other of this.views) if (other !== view) other.dispatch({ changes: transaction.changes, annotations: [sync.of(true), Transaction.userEvent.of(transaction.annotation(Transaction.userEvent) ?? 'input')] })
      this.workspace.edit(this.doc, view.state.doc.toString())
    }
    this.state = this.primary?.state ?? view.state
  }
  private reconfigure(effect: ReturnType<Compartment['reconfigure']>): void {
    this.state = this.state.update({ effects: effect }).state
    for (const view of this.views) view.dispatch({ effects: effect })
  }
  configure(): void {
    const d = this.doc; const w = this.workspace; const s = w.settings
    const key = JSON.stringify([s, w.theme, d.readOnly, d.mutating, d.name, d.largeFile])
    if (key !== this.configKey) {
      this.configKey = key
      const indentation = d.text.match(/^([ \t]+)\S/m)?.[1]
      const unit = indentation?.startsWith('\t') ? '\t' : indentation && indentation.length <= 8 ? indentation : s.insertSpaces ? ' '.repeat(s.tabSize) : '\t'
      const wrap = s.wordWrap === 'on' || s.wordWrap === 'auto' && /\.(md|markdown|txt|text)$/i.test(d.name)
      const extensions: Extension[] = [EditorState.allowMultipleSelections.of(true), EditorState.readOnly.of(d.readOnly || !!d.mutating), EditorView.editable.of(!d.readOnly && !d.mutating),
        EditorState.tabSize.of(s.tabSize), indentUnit.of(unit), drawSelection(), rectangularSelection(), search({ top: true }),
        EditorView.theme({ '&': { height: '100%', fontSize: `${s.fontSize}px` }, '.cm-scroller': { overflow: 'auto', fontFamily: 'ui-monospace, SFMono-Regular, Consolas, monospace' }, '.cm-content': { padding: '12px 0' }, '.cm-gutters': { backgroundColor: 'var(--panel)', color: 'var(--muted)', border: 'none' }, '&.cm-focused': { outline: 'none' }, '.cm-activeLine': { backgroundColor: 'var(--active)' }, '.cm-cursor': { borderLeftColor: 'var(--fg)' }, '&.cm-focused .cm-selectionBackground, .cm-selectionBackground': { background: 'var(--selection)' } }, { dark: w.theme === 'dark' }),
        keymap.of([{ key: 'Mod-z', run: () => this.primary ? undo(this.primary) : false }, { key: 'Mod-Shift-z', run: () => this.primary ? redo(this.primary) : false }, { key: 'Mod-y', run: () => this.primary ? redo(this.primary) : false }, { key: 'Mod-h', run: openSearchPanel }, { key: 'Mod-g', run: gotoLine }, { key: 'Mod-d', run: selectNextOccurrence }, ...defaultKeymap, ...searchKeymap, ...historyKeymap, indentWithTab])]
      if (s.lineNumbers) extensions.push(lineNumbers())
      if (s.renderWhitespace) extensions.push(highlightWhitespace())
      if (wrap) extensions.push(EditorView.lineWrapping)
      if (!d.largeFile) extensions.push(highlightActiveLine(), foldGutter(), bracketMatching(), indentOnInput(), syntaxHighlighting(defaultHighlightStyle), closeBrackets(), keymap.of(closeBracketsKeymap))
      this.reconfigure(this.config.reconfigure(extensions))
    }
    const languageKey = `${d.name}:${d.language}:${d.largeFile}`
    if (languageKey !== this.languageKey) {
      this.languageKey = languageKey; const serial = ++this.serial
      void (d.largeFile ? Promise.resolve(undefined) : languageFor(d.name, d.language, d.source)).then(language => {
        if (serial === this.serial) { this.reconfigure(this.language.reconfigure(language ?? [])); w.emit() }
      }).catch(e => w.notify(String(e)))
    }
    if (this.state.doc.toString() !== d.text) {
      for (const view of this.views) view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: d.text }, annotations: [sync.of(true), Transaction.addToHistory.of(false)] })
      this.state = this.primary?.state ?? this.state.update({ changes: { from: 0, to: this.state.doc.length, insert: d.text }, annotations: [sync.of(true), Transaction.addToHistory.of(false)] }).state
    }
  }
}
