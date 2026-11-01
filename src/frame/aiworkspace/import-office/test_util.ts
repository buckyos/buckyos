/* Helpers of the Office importer's tests: assertions, fixture paths, golden files (`OFFICE_REGEN=1 deno test -A`
 * rewrites ../fixtures/office/expected/*.plan.json), a deterministic id source, and LibreOffice when it is there. */

export const here = new URL('.', import.meta.url).pathname
export const FIXTURES = `${here}../fixtures/office/`
const REGEN = Deno.env.get('OFFICE_REGEN') === '1'

export function assert(cond: unknown, message = 'assertion failed'): asserts cond {
  if (!cond) throw new Error(message)
}
export function equal(a: unknown, b: unknown, message = '') {
  const x = JSON.stringify(a)
  const y = JSON.stringify(b)
  if (x !== y) throw new Error(`${message}\n  actual:   ${x.slice(0, 800)}\n  expected: ${y.slice(0, 800)}`)
}

export const fixture = (path: string) => Deno.readFile(`${FIXTURES}${path}`)

/** Ids `b1`, `b2` … in call order (stable plans). */
export function counter(): (prefix: string) => string {
  let n = 0
  return (prefix) => `${prefix}${++n}`
}

export async function golden(name: string, got: unknown) {
  const file = `${FIXTURES}expected/${name}.plan.json`
  const text = JSON.stringify(got, (_k, v) => (v instanceof Uint8Array ? `<${v.length} bytes>` : v), 2) + '\n'
  if (REGEN) { await Deno.writeTextFile(file, text); return }
  let want: string
  try { want = await Deno.readTextFile(file) } catch { throw new Error(`${name}: no golden plan (OFFICE_REGEN=1 to write it)`) }
  if (want !== text) equal(JSON.parse(text), JSON.parse(want), `${name} plan differs (OFFICE_REGEN=1 to accept)`)
}

/** The LibreOffice binary when installed (tests of sub-documents convert them when it is). */
export async function soffice(): Promise<string | null> {
  for (const p of ['/usr/bin/soffice', '/usr/lib/libreoffice/program/soffice', '/Applications/LibreOffice.app/Contents/MacOS/soffice']) {
    try { await Deno.stat(p); return p } catch { /* next */ }
  }
  return null
}

/** Convert a document to PDF with LibreOffice; the PDF's bytes, or null when it failed. */
export async function toPdf(bin: string, bytes: Uint8Array, ext: string): Promise<Uint8Array | null> {
  const dir = await Deno.makeTempDir({ prefix: 'office-test-' })
  try {
    await Deno.writeFile(`${dir}/input.${ext}`, bytes)
    const out = await new Deno.Command(bin, { args: [`-env:UserInstallation=file://${dir}/profile`, '--headless', '--norestore', '--nologo', '--nolockcheck', '--nodefault', '--convert-to', 'pdf', '--outdir', dir, `${dir}/input.${ext}`], stdout: 'null', stderr: 'null' }).output()
    if (!out.success) return null
    return await Deno.readFile(`${dir}/input.pdf`).catch(() => null)
  } finally {
    await Deno.remove(dir, { recursive: true }).catch(() => {})
  }
}
