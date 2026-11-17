/* One printed page (打印 TODO §3.2, §3.3): a box of the page's size, its background, the content, and — with a PDF Frame
 * — its header and footer drawn over the page (Q19: Chromium's margins and templates are not used). The theme follows
 * the page background, not the Desktop's (§3.2). */

import type { ReactNode } from 'react'
import { PX_PER_MM, type PageContext, type Paper } from '@print/page.ts'
import { stageTheme } from '../../presentation/model'
import { PageContextValue, PdfFrameDecoration, type PdfFrameInfo } from '../../ui/controls/pdfframe/api'

export interface PrintPageProps {
  paper: Paper
  frame: PdfFrameInfo | null
  context: PageContext
  background: string
  children: ReactNode
}

export function PrintPage({ paper, frame, context, background, children }: PrintPageProps) {
  const theme = stageTheme(background)
  return (
    <section className={`aiws-print-page cp-theme-${theme}`} data-page={context.page} data-theme={theme}
      style={{ width: `${paper.width_mm}mm`, height: `${paper.height_mm}mm`, background, ['--aiws-print-bg' as string]: background }}>
      {children}
      {frame && (
        <div className="aiws-print-decoration">
          <PageContextValue.Provider value={context}>
            <PdfFrameDecoration frameId={frame.id} widthPx={paper.width_mm * PX_PER_MM} heightPx={paper.height_mm * PX_PER_MM} context={context} />
          </PageContextValue.Provider>
        </div>
      )}
    </section>
  )
}
