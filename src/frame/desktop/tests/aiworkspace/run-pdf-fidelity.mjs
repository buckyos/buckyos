import { spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const repo = resolve(desktop, '../../..')
const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    help: { type: 'boolean' },
    pages: { type: 'string' },
    output: { type: 'string' },
    'reference-dir': { type: 'string' },
    'scratch-dir': { type: 'string' },
    'analyze-all': { type: 'boolean' },
    'decoder-control': { type: 'boolean' },
    'decoder-page': { type: 'string' },
    'control-only': { type: 'boolean' },
    project: { type: 'string', default: 'both' },
  },
})
if (values.help) {
  console.log(`Usage: pnpm test:pdf [synthetic-default.pdf | /absolute/source.pdf] [--pages 1,2,3,4] [--output DIR]
  --pages LIST        Compare the first four available pages by default
  --reference-dir DIR  Use trusted page-N.png references instead of Poppler
  --scratch-dir DIR   Short temporary path for browser profiles (or TMPDIR)
  --analyze-all        Record operator/text inventory for every source page
  --decoder-control   Add the image decoder WASM A/B check (default for synthetic input)
  --decoder-page N    Image page for the decoder control (default 2)
  --control-only      Run only that A/B check
  --project MODE      editable, raster, or both (default)
Requires prebuilt AIWS_BIN/NFS_BIN, existing Playwright Chromium and pdftoppm.`)
  process.exit(0)
}
if (positionals.length > 1) throw new Error('Provide at most one local PDF path; see --help')
const input = resolve(positionals[0] ?? join(repo, 'src/frame/aiworkspace/fixtures/pdf/synthetic-pdf-import.pdf'))
if (!existsSync(input)) throw new Error(`PDF does not exist: ${input}`)
const decoderPage = Number(values['decoder-page'] ?? 2)
if (!Number.isSafeInteger(decoderPage) || decoderPage < 1) throw new Error('Decoder page must be a positive integer')
if (!['editable', 'raster', 'both'].includes(values.project)) throw new Error('Project must be editable, raster, or both')
const decoderRequested = values['decoder-control'] || values['control-only'] || values['decoder-page'] !== undefined
if (decoderRequested && values.project === 'raster') throw new Error('Decoder control requires the editable project')
const env = { ...process.env }
let pages
if (values['control-only']) pages = [decoderPage]
else if (values.pages) pages = values.pages.split(',').map(Number)
else if (!positionals.length) pages = [1, 2, 3, 4]
else {
  const result = spawnSync('pdfinfo', [input], { env: { ...env, LC_ALL: 'C' }, encoding: 'utf8' })
  const count = Number(/^Pages:\s+(\d+)/m.exec(result.stdout ?? '')?.[1])
  if (result.error || result.status !== 0 || !Number.isSafeInteger(count) || count < 1) {
    throw new Error(`Cannot read page count (use --pages to select pages explicitly): ${result.error?.message ?? result.stderr}`)
  }
  pages = Array.from({ length: Math.min(count, 4) }, (_, index) => index + 1)
}
if (!pages.length || pages.some(n => !Number.isSafeInteger(n) || n < 1)) throw new Error('Pages must be positive integers')
if (decoderRequested && !pages.includes(decoderPage)) pages.push(decoderPage)
const decoderControl = decoderRequested || (!positionals.length && values.project !== 'raster' && pages.includes(decoderPage))
if (!positionals.length && pages.some(n => n > 4)) throw new Error('The built-in synthetic PDF has four pages')
for (const [name, fallback] of [
  ['AIWS_BIN', 'src/rootfs/bin/aiworkspace/aiworkspace'],
  ['NFS_BIN', 'src/rootfs/bin/nfs-server/nfs_server'],
]) {
  env[name] = resolve(env[name] ?? join(repo, fallback))
  if (!existsSync(env[name])) throw new Error(`${name} must point to a prebuilt isolated-test binary`)
}
const output = resolve(values.output ?? join(repo, '..', '.work', `pdf-fidelity-${Date.now()}`))
const reference = resolve(values['reference-dir'] ?? join(output, 'reference'))
if (reference === output || reference.startsWith(join(output, 'browser') + '/')) throw new Error('Reference directory must be separate from browser output')
mkdirSync(reference, { recursive: true })
const scratch = resolve(values['scratch-dir'] ?? env.TMPDIR ?? join(output, 'tmp'))
mkdirSync(scratch, { recursive: true })
env.TMPDIR = env.TMP = env.TEMP = scratch
env.BUCKYOS_ROOT = join(output, 'isolated-runtime')
Object.assign(env, {
  AIWS_PDF_FILE: input,
  AIWS_PDF_PROFILE: positionals.length ? '' : join(repo, 'src/frame/aiworkspace/fixtures/pdf/fidelity.manifest.json'),
  AIWS_PDF_PAGES: [...new Set(pages)].join(','),
  AIWS_PDF_ANALYZE_ALL: values['analyze-all'] ? '1' : '0',
  AIWS_PDF_DECODER_CONTROL: decoderControl ? '1' : '0',
  AIWS_PDF_DECODER_PAGE: String(decoderPage),
  AIWS_PDF_REFERENCE_DIR: reference,
  AIWS_PDF_OUTPUT: join(output, 'browser'),
})
if (!values['reference-dir']) {
  for (const page of new Set(pages)) {
    const result = spawnSync('pdftoppm', ['-f', String(page), '-l', String(page), '-singlefile', '-scale-to', '1200', '-png', input, join(reference, `page-${page}`)], { env, encoding: 'utf8' })
    if (result.error || result.status !== 0) throw new Error(`Independent reference rendering failed: ${result.error?.message ?? result.stderr}`)
  }
}
for (const page of pages) if (!existsSync(join(reference, `page-${page}.png`))) throw new Error(`Missing reference page-${page}.png`)
const args = ['exec', 'playwright', 'test', '--config', 'playwright.pdf.config.ts']
if (values['control-only']) args.push('--project', 'editable', '--grep', 'decoder asset')
else if (values.project !== 'both') args.push('--project', values.project)
const started = Date.now()
const result = spawnSync(process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm', args, { cwd: desktop, env, stdio: 'inherit' })
const status = { input, pages, reference, output, seconds: (Date.now() - started) / 1000, exitCode: result.status, error: result.error?.message, productionData: false, externalModelCalls: 0 }
writeFileSync(join(output, 'run.json'), JSON.stringify(status, null, 2))
console.log(`PDF fidelity report: ${join(output, 'browser/html/index.html')}`)
if (result.error) throw result.error
process.exitCode = result.status ?? 1
