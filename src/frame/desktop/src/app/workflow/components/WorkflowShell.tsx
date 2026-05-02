/* ── Workflow shell – responsive layout ── */

import { useCallback, useState } from 'react'
import { useMediaQuery } from '@mui/material'
import { Menu, X } from 'lucide-react'
import { useMobileBackHandler } from '../../../desktop/windows/MobileNavContext'
import { WorkflowSidebar } from './Sidebar'
import { ImportDialog } from './ImportDialog'
import { HomePage } from '../pages/HomePage'
import { DetailPage } from '../pages/DetailPage'
import { AIPromptPage } from '../pages/AIPromptPage'
import type { WorkflowSelection } from '../mock/types'

export function WorkflowShell() {
  const [selection, setSelection] = useState<WorkflowSelection>({ kind: 'home' })
  const [importOpen, setImportOpen] = useState(false)
  const [drawerOpen, setDrawerOpen] = useState(false)
  const isMobile = useMediaQuery('(max-width: 767px)')

  const closeDrawer = useCallback(() => setDrawerOpen(false), [])

  useMobileBackHandler(
    isMobile && drawerOpen
      ? closeDrawer
      : isMobile && selection.kind !== 'home'
        ? () => setSelection({ kind: 'home' })
        : null,
  )

  function navigate(s: WorkflowSelection) {
    setSelection(s)
    if (isMobile) setDrawerOpen(false)
  }

  function renderMain() {
    switch (selection.kind) {
      case 'home':
        return (
          <div className="px-5 py-5 max-w-5xl">
            <HomePage onSelect={navigate} onImport={() => setImportOpen(true)} />
          </div>
        )
      case 'ai_prompt':
        return (
          <div className="px-5 py-5 max-w-4xl">
            <AIPromptPage onImport={() => setImportOpen(true)} />
          </div>
        )
      case 'definition':
      case 'mount':
        return <DetailPage selection={selection} onSelect={navigate} />
    }
  }

  return (
    <div className="flex h-full w-full flex-col" style={{ background: 'var(--cp-bg)' }}>
      {isMobile && (
        <div
          className="flex items-center gap-2 px-3 py-2"
          style={{ borderBottom: '1px solid var(--cp-border)' }}
        >
          <button
            type="button"
            onClick={() => setDrawerOpen(true)}
            className="flex h-7 w-7 items-center justify-center rounded-lg"
            style={{
              background: 'var(--cp-surface-2)',
              color: 'var(--cp-text)',
              border: '1px solid var(--cp-border)',
            }}
          >
            <Menu size={14} />
          </button>
          <div className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
            Workflow
          </div>
        </div>
      )}

      <div className="flex flex-1 min-h-0">
        {!isMobile && (
          <WorkflowSidebar
            selection={selection}
            onSelect={navigate}
            onImport={() => setImportOpen(true)}
          />
        )}
        <main className="flex-1 min-w-0 overflow-hidden desktop-scrollbar">
          <div className="flex h-full min-h-0 flex-col overflow-y-auto">
            {renderMain()}
          </div>
        </main>
      </div>

      {isMobile && drawerOpen && (
        <div
          className="fixed inset-0 z-40 flex"
          style={{ background: 'rgba(0,0,0,0.4)' }}
          onClick={closeDrawer}
        >
          <div
            className="h-full w-[300px] max-w-[85vw]"
            style={{ background: 'var(--cp-bg)' }}
            onClick={(e) => e.stopPropagation()}
          >
            <div
              className="flex items-center justify-end px-3 py-2"
              style={{ borderBottom: '1px solid var(--cp-border)' }}
            >
              <button
                type="button"
                onClick={closeDrawer}
                className="flex h-7 w-7 items-center justify-center rounded-lg"
                style={{ color: 'var(--cp-muted)' }}
              >
                <X size={14} />
              </button>
            </div>
            <WorkflowSidebar
              selection={selection}
              onSelect={navigate}
              onImport={() => {
                setImportOpen(true)
                closeDrawer()
              }}
            />
          </div>
        </div>
      )}

      {importOpen && (
        <ImportDialog
          onClose={() => setImportOpen(false)}
          onShowAiPrompt={() => {
            setImportOpen(false)
            navigate({ kind: 'ai_prompt' })
          }}
          onImported={(definitionId) => {
            setImportOpen(false)
            navigate({ kind: 'definition', definitionId })
          }}
        />
      )}
    </div>
  )
}
