/* How a PDF Frame's page shape reads in the interface (the properties, the content placeholder, the print dialog). */

import { FRAME_PAGES, type FramePage, type PdfFrameConfig } from '@controls/pdfframe'
import { aiwsT, localizedLabels } from '../../../i18n'

const PAGE_LABEL = localizedLabels<FramePage>({
  A: 'aiworkspace.pdfframe.page.A', Letter: 'aiworkspace.pdfframe.page.Letter', '16:9': 'aiworkspace.pdfframe.page.wide',
  '4:3': 'aiworkspace.pdfframe.page.standard', custom: 'aiworkspace.pdfframe.page.custom',
})
const ORIENTATION_LABEL = localizedLabels<PdfFrameConfig['orientation']>({
  portrait: 'aiworkspace.pdfframe.orientation.portrait', landscape: 'aiworkspace.pdfframe.orientation.landscape',
})

/** What a page context value is called in a label's menu. */
export const PAGE_VALUE_LABEL = localizedLabels({
  page: 'aiworkspace.pdfframe.value.page', pages: 'aiworkspace.pdfframe.value.pages', title: 'aiworkspace.pdfframe.value.title', date: 'aiworkspace.pdfframe.value.date',
})

export const pageLabel = (page: FramePage) => PAGE_LABEL[page]
export const orientationLabel = (o: PdfFrameConfig['orientation']) => ORIENTATION_LABEL[o]

/** "A series · Portrait", "Custom 100 × 50 mm · Landscape". */
export function shapeLabel(c: PdfFrameConfig): string {
  const page = c.page === 'custom' ? aiwsT('aiworkspace.pdfframe.customSize', { w: c.custom_w, h: c.custom_h }) : pageLabel(c.page)
  return `${page} · ${orientationLabel(c.orientation)}`
}

export const pageOptions = () => FRAME_PAGES.map((p) => ({ value: p, label: pageLabel(p) }))
