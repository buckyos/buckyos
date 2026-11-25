/* The fixed-format messages the App writes to the browser conversation for the personal site. Each is the owner's own
 * message: the first line says what is wanted, the lines under it are `Name: value`. The Agent's manuals (`browser`,
 * `reading_site`) quote the first lines word for word, so they are English whatever the App's language, and are
 * changed together with the manuals. */

/** The owner wants a website collected into the personal site and kept up to date. */
export const ADD_SOURCE_REQUEST = 'Please add this website to my reading site.'
/** The owner agrees to an upgrade request of the site's maintenance Goal. */
export const UPGRADE_REQUEST = 'Please go ahead with this upgrade of my reading site.'
/** The owner wants one page tidied, without the website becoming a source. */
export const PAGE_ONLY_REQUEST = 'Please tidy this page only: make a one-off reflow page for it (reflow result put), without adding the website to my reading site.'
/** The owner wants a source gone from the personal site: its program, its job and its place in the maintenance Goal. */
export const REMOVE_SOURCE_REQUEST = 'Please remove this website from my reading site.'

const lines = (head: string, fields: Array<[name: string, value: string]>): string =>
  [head, ...fields.filter(([, value]) => value).map(([name, value]) => `${name}: ${value}`)].join('\n')

/** One line, so a field never runs into the next one. */
const oneLine = (value: string): string => value.replace(/\s+/g, ' ').trim()

/** `note`: the one line the owner added, or empty. */
export function addSourceMessage({ url, title, note }: { url: string; title: string; note: string }): string {
  return lines(ADD_SOURCE_REQUEST, [['URL', url], ['Title', oneLine(title)], ['Note', oneLine(note)]])
}

/** `file`: the request's write-up in the maintainer's workspace; `goalId`: the Goal that asked. */
export function upgradeMessage({ id, file, goalId }: { id: string; file: string; goalId: string }): string {
  return lines(UPGRADE_REQUEST, [['Upgrade', id], ['File', file], ['Goal', goalId]])
}

/** `note`: the one line the owner added, or empty. */
export function pageOnlyMessage({ url, title, note }: { url: string; title: string; note: string }): string {
  return lines(PAGE_ONLY_REQUEST, [['URL', url], ['Title', oneLine(title)], ['Note', oneLine(note)]])
}

/** `id`: the source's id in `site.json`; `url`: its `home_url`. */
export function removeSourceMessage({ id, title, url }: { id: string; title: string; url: string }): string {
  return lines(REMOVE_SOURCE_REQUEST, [['Source', id], ['Title', oneLine(title)], ['URL', url]])
}
