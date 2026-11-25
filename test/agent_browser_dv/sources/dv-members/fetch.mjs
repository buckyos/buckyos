// Runs in a background tab of the Agent's browser (browser.background_run), for its cookies only: the pages
// are plain HTML once the user is signed in, so they are fetched with `request` and nothing is rendered.
// A page that sends the reader to sign in ends the run as "auth".
export default async ({ request, args }) => {
  const response = await request.get(args.url, { maxRedirects: 0 })
  const status = response.status()
  if (status === 401 || status === 403 || (status >= 300 && status < 400)) {
    throw Object.assign(new Error(`${args.url} asks to sign in (HTTP ${status})`), { code: 'auth' })
  }
  return { status, url: response.url(), html: await response.text() }
}
