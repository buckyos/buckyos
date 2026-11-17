/* What a PDF Frame draws on a canvas under its header and footer Blocks (打印 TODO §3.3): the page, its two bands, and
 * the content area as a placeholder (printing puts the page's content there), with the page shape written in it. */

import { useAiwsText } from '../../../useAiwsText'
import type { ControlChromeProps } from '../registry'
import { frameLayout } from './model'
import { shapeLabel } from './labels'
import './pdfframe.css'

export function PdfFrameChrome({ container, mode }: ControlChromeProps) {
  const { t } = useAiwsText()
  const id = container.entity.entity_id
  const { config, areas } = frameLayout(container.entity)
  const box = container.rect
  const editing = mode === 'edit' || mode === 'presentation_edit'
  const at = (r: { x: number; y: number; w: number; h: number }) => ({ left: r.x, top: r.y, width: Math.max(0, r.w), height: Math.max(0, r.h) })
  return (
    <div className="aiws-pdfframe-chrome" data-testid={`aiws-pdfframe-${id}`} data-page={config.page} data-orientation={config.orientation}
      style={{ width: Math.max(1, box.w), height: Math.max(1, box.h) }}>
      <div className="aiws-pdfframe-band is-header" data-testid={`aiws-pdfframe-header-${id}`} style={at(areas.header)} />
      <div className="aiws-pdfframe-band is-footer" data-testid={`aiws-pdfframe-footer-${id}`} style={at(areas.footer)} />
      <div className="aiws-pdfframe-content" data-testid={`aiws-pdfframe-content-${id}`} style={at(areas.content)}>
        <span className="aiws-pdfframe-content-title">{t('aiworkspace.pdfframe.content')}</span>
        <span className="aiws-pdfframe-content-shape">{shapeLabel(config)}</span>
      </div>
      {editing && areas.header.h > 0 && <span className="aiws-pdfframe-tag" style={{ top: areas.header.y }}>{t('aiworkspace.pdfframe.header')}</span>}
      {editing && areas.footer.h > 0 && <span className="aiws-pdfframe-tag" style={{ top: areas.footer.y }}>{t('aiworkspace.pdfframe.footer')}</span>}
    </div>
  )
}
