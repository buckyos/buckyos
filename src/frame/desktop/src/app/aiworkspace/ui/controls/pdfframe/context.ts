/* The page context a PDF Frame's labels read (打印 TODO §3.3): `{page}`, `{pages}`, `{title}`, `{date}` in a label's text.
 * Printing provides the values of each page (`PageContextValue`); on a canvas there is none and the labels of a frame show
 * samples: page 1 of 12, the title of the canvas the frame is on, today. */

import { createContext } from 'react'
import { SAMPLE_PAGE_CONTEXT, type PageContext } from '@print/page.ts'
import { aiwsT, aiwsText } from '../../../i18n'

/** `null`: the frame is edited on a canvas, its labels show sample values. */
export const PageContextValue = createContext<PageContext | null>(null)

export const PAGE_VALUES = ['page', 'pages', 'title', 'date'] as const
export type PageValue = (typeof PAGE_VALUES)[number]

/** A day as a page's `{date}` shows it, in the app's language (a long date: 2026年11月16日, November 16, 2026). */
export function formatPageDate(date: Date = new Date()): string {
  return new Intl.DateTimeFormat(aiwsText().intl, { year: 'numeric', month: 'long', day: 'numeric' }).format(date)
}

/** The values a frame's labels show while the frame is edited: `title` is the canvas's (a default when it has none). */
export function samplePageContext(canvasTitle: string | null | undefined): PageContext {
  return { ...SAMPLE_PAGE_CONTEXT, title: canvasTitle || aiwsT('aiworkspace.pdfframe.sampleTitle'), date: formatPageDate() }
}
