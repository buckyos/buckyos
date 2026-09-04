/* ── BuckyOS AI Canvas – app panel entry point ── */

import { useEffect, useMemo, useState } from 'react'

import './canvas.css'
import { WishRunner } from './agent/runner'
import type { CanvasDocument } from './domain/types'
import { CanvasStore } from './store/canvas-store'
import { CanvasEditorContext } from './store/hooks'
import { IndexedDbCanvasStorage, type CanvasStorageAdapter } from './storage/indexeddb'
import { EditorShell } from './ui/EditorShell'
import { HomePage } from './ui/HomePage'

type View = { kind: 'home' } | { kind: 'editor'; doc: CanvasDocument; importFile?: File }

export function CanvasAppPanel() {
  const [storage] = useState<CanvasStorageAdapter>(() => new IndexedDbCanvasStorage())
  const [view, setView] = useState<View>({ kind: 'home' })

  if (view.kind === 'home') {
    return <HomePage storage={storage} onOpen={(doc, opts) => setView({ kind: 'editor', doc, importFile: opts?.importFile })} />
  }
  return <EditorHost key={view.doc.id} doc={view.doc} storage={storage} importFile={view.importFile} onBack={() => setView({ kind: 'home' })} />
}

function EditorHost({ doc, storage, importFile, onBack }: { doc: CanvasDocument; storage: CanvasStorageAdapter; importFile?: File; onBack: () => void }) {
  const value = useMemo(() => {
    const store = new CanvasStore(doc, storage)
    const runner = new WishRunner(store)
    return { store, runner }
  }, [doc, storage])
  useEffect(() => () => value.store.dispose(), [value])
  return (
    <CanvasEditorContext.Provider value={value}>
      <EditorShell onBack={onBack} storage={storage} pendingImportFile={importFile ?? null} />
    </CanvasEditorContext.Provider>
  )
}
