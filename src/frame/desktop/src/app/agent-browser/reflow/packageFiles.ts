/* A reflow package handed to the App (RF7): its files by their path inside the package, from a zip or from a
 * folder of the Zone (where an Agent that cannot write the library leaves the package it wrote). Both leave out
 * what `reflow put` leaves out of a directory: hidden entries. The zip reader is the table importer's (stored and
 * deflated entries). */

import { isZip, zipEntries, zipRead, XlsxError, type InflateBudget } from '@table/xlsx'
import { MANIFEST_FILE, MAX_FILE_BYTES, MAX_PACKAGE_FILES, ReflowError } from './library'

/** What a package may come to in all. */
const MAX_PACKAGE_BYTES = 256 * 1024 * 1024

function invalid(problem: string): ReflowError {
  return new ReflowError('the package is not valid', [problem])
}

const hidden = (name: string) => name.split('/').some((segment) => segment.startsWith('.'))

function checkCount(count: number) {
  if (count > MAX_PACKAGE_FILES) throw invalid(`a package has at most ${MAX_PACKAGE_FILES} files, this one has ${count}`)
}

function checkSize(name: string, size: number, budget: InflateBudget) {
  if (size > MAX_FILE_BYTES) throw invalid(`${name} is ${size} bytes; a package file is at most ${MAX_FILE_BYTES}`)
  budget.left -= size
  if (budget.left < 0) throw invalid('the package is too large')
}

/**
 * The files of the package in a zip. The package is the zip's content, or the one folder the zip holds when the
 * manifest is in it (a zipped directory).
 */
export async function packageFilesOfZip(bytes: Uint8Array): Promise<Map<string, Uint8Array>> {
  if (!isZip(bytes)) throw invalid('the file is not a zip archive')
  let entries: ReturnType<typeof zipEntries>
  try {
    entries = zipEntries(bytes)
  } catch {
    throw invalid('the zip archive is damaged')
  }
  // Folders, hidden files and what macOS adds to an archive are no package files.
  const names = [...entries.keys()].filter((name) => !name.endsWith('/') && !name.startsWith('__MACOSX/') && !hidden(name))
  for (const name of names) {
    if (name.includes('\\') || name.split('/').some((segment) => !segment)) throw invalid(`${name} is not a file path inside the package`)
  }
  checkCount(names.length)
  const folder = names[0]?.includes('/') ? names[0].slice(0, names[0].indexOf('/') + 1) : ''
  const strip = !entries.has(MANIFEST_FILE) && folder && names.every((name) => name.startsWith(folder)) && names.includes(`${folder}${MANIFEST_FILE}`) ? folder.length : 0
  const declared: InflateBudget = { left: MAX_PACKAGE_BYTES }
  const budget: InflateBudget = { left: MAX_PACKAGE_BYTES }
  const files = new Map<string, Uint8Array>()
  for (const name of names) {
    const entry = entries.get(name)!
    checkSize(name, entry.size, declared)
    try {
      files.set(name.slice(strip), await zipRead(bytes, entry, budget))
    } catch (error) {
      const code = error instanceof XlsxError ? error.code : 'corrupt'
      throw invalid(code === 'encrypted' ? 'the zip archive is encrypted' : code === 'unsupported' ? `${name} uses a compression this App cannot read` : code === 'too_large' ? 'the package is too large' : 'the zip archive is damaged')
    }
  }
  return files
}

/** The files of the package in a folder: `files` are all the files below it, each with `webkitRelativePath` starting with the folder's name. */
export async function packageFilesOfFolder(folder: File[]): Promise<Map<string, Uint8Array>> {
  const named = folder
    .map((file) => ({ file, name: file.webkitRelativePath.slice(file.webkitRelativePath.indexOf('/') + 1) }))
    .filter(({ name }) => !hidden(name))
  checkCount(named.length)
  const budget: InflateBudget = { left: MAX_PACKAGE_BYTES }
  const files = new Map<string, Uint8Array>()
  for (const { file, name } of named) {
    checkSize(name, file.size, budget)
    files.set(name, new Uint8Array(await file.arrayBuffer()))
  }
  return files
}
