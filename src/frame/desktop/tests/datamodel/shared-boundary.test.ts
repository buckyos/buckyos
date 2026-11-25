// src/shared is used by the Shell and, by path, by the BuckyOS APP client: it may import only the `buckyos` package and
// its own files (relative, with the extension), and holds nothing of the Shell (src/shared/channel.ts).

const root = new URL('../../src/shared/', import.meta.url)

async function* files(dir: URL): AsyncGenerator<URL> {
  for await (const entry of Deno.readDir(dir)) {
    const url = new URL(entry.name + (entry.isDirectory ? '/' : ''), dir)
    if (entry.isDirectory) yield* files(url)
    else yield url
  }
}

const IMPORTS = /(?:^|[\s;])(?:import|export)\b[^'"]*?\bfrom\s*['"]([^'"]+)['"]|\bimport\s*\(?\s*['"]([^'"]+)['"]/g

Deno.test('src/shared imports only the buckyos package and its own files', async () => {
  const problems: string[] = []
  for await (const url of files(root)) {
    const name = url.pathname.slice(root.pathname.length)
    if (!name.endsWith('.ts')) {
      if (!name.endsWith('.css')) problems.push(`${name}: only .ts and .css files`)
      continue
    }
    const source = await Deno.readTextFile(url)
    for (const [, from, bare] of source.matchAll(IMPORTS)) {
      const specifier = from ?? bare
      if (specifier === 'buckyos' || specifier.startsWith('buckyos/')) continue
      const target = new URL(specifier, url)
      if (!/^\.\.?\//.test(specifier) || !specifier.endsWith('.ts')) problems.push(`${name}: import '${specifier}'`)
      else if (!target.pathname.startsWith(root.pathname)) problems.push(`${name}: import '${specifier}' leaves src/shared`)
    }
    if (/\bimport\.meta\.env\b/.test(source)) problems.push(`${name}: import.meta.env`)
    if (/import\s*\{[^}]*\bbuckyos\b[^}]*\}\s*from\s*['"]buckyos['"]/.test(source)) problems.push(`${name}: the SDK's global runtime`)
    if (/\b(?:localStorage|sessionStorage|indexedDB)\b/.test(source)) problems.push(`${name}: browser storage`)
  }
  if (problems.length > 0) throw new Error(problems.join('\n'))
})

Deno.test('the design tokens are plain CSS', async () => {
  const source = await Deno.readTextFile(new URL('tokens.css', root))
  const directive = /@(?:tailwind|apply|layer|config|screen|import)\b/.exec(source)
  if (directive) throw new Error(`tokens.css has ${directive[0]}`)
})
