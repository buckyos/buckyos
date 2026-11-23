/* The print renderer (打印 TODO T7, Q18–Q20): the entry of `aiws-print/print.js`, a classic script the aiworkspace
 * service puts into every print package next to `data.js` (`window.__AIWS_PRINT__`). `media.page` opens the package
 * over file:// in Chromium and prints once `window.__AIWS_PRINT_READY__` is set: here, when every page is drawn, or at
 * the latest after OVERALL_MS (what is not done by then prints as it is). Built by vite.config.ts
 * (`aiwsPrintRenderer`), with every WASM inlined. */

import { createRoot } from 'react-dom/client'
import { PRINT_FORMAT, type PrintPackage } from '@print/package.ts'
import '../../../../index.css'
import '../../aiworkspace.css'
import { I18nProvider } from '../../../../i18n/provider'
import { supportedLocales, type SupportedLocale } from '../../../../models/ui'
import { loadCore } from '../../api/wasm'
import { loadAiwsLocale, setAiwsLocale } from '../../i18n'
import { loadLoro } from '../../richtext/loro'
import { WorkspaceStore } from '../../state/store'
import { registerDefaultBlocks } from '../../ui/blocks/registerAll'
import { PrintApp } from './PrintApp'
import { PrintSession } from './session'
import './print.css'

const OVERALL_MS = 45_000
/** The flow's measuring pass gets this much of it; the pages the rest. */
const MEASURE_MS = 25_000

function ready() {
  window.__AIWS_PRINT_READY__ = true
}

/** A package this renderer cannot print never becomes ready: the run fails, and no half-made PDF is written (§3.4). */
function fail(message: string) {
  const note = document.createElement('pre')
  note.className = 'aiws-print-failure'
  note.textContent = message
  document.body.appendChild(note)
}

async function start(pkg: PrintPackage) {
  const started = performance.now()
  const locale: SupportedLocale = (supportedLocales as readonly string[]).includes(pkg.job.locale) ? pkg.job.locale as SupportedLocale : 'en'
  setAiwsLocale(locale)
  const text = loadAiwsLocale(locale)
  document.documentElement.lang = locale
  document.title = pkg.job.title
  // the page size of every page: Chromium's margins are 0, the header and footer are ours (Q19)
  const page = document.createElement('style')
  page.textContent = `@page { size: ${pkg.job.paper.width_mm}mm ${pkg.job.paper.height_mm}mm; margin: 0; }`
  document.head.appendChild(page)
  const [core] = await Promise.all([loadCore(), loadLoro(), text])
  registerDefaultBlocks()
  const store = new WorkspaceStore(new PrintSession(core, pkg), core)
  await store.outline.reload()
  const timer = window.setTimeout(ready, Math.max(0, started + OVERALL_MS - performance.now()))
  const failed = (message: string) => { window.clearTimeout(timer); fail(message) }
  const host = document.getElementById('aiws-print') ?? document.body.appendChild(document.createElement('div'))
  createRoot(host).render(
    <I18nProvider locale={locale}>
      <PrintApp store={store} job={pkg.job} measureDeadline={started + MEASURE_MS} deadline={started + OVERALL_MS - 1000} onReady={ready} onFail={failed} />
    </I18nProvider>,
  )
}

const pkg = window.__AIWS_PRINT__
if (!pkg || pkg.format !== PRINT_FORMAT) fail('aiws-print: no print package (data.js)')
else start(pkg).catch((error: unknown) => { console.error('[aiws-print]', error); fail(error instanceof Error ? error.message : String(error)) })
