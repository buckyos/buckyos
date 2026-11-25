# dv-members — a source that needs the owner's sign-in

The members' column `/members` of the DV test site (`test/agent_browser_dv/site.mjs`). Its pages are ordinary HTML, but only for a signed-in reader: without the session cookie (`dvsid`) every address under `/members` answers `303` to `/login`. So `source.json` says `"needs": "login"`: the pages are read with the cookies the owner's sign-in left in the Agent's browser. Nothing here knows the password or exports a cookie.

## How it is read

1. `ctx.browser(null, "fetch.mjs", { url })` fetches one address with `request` of the background tab (the browser's cookies, no page rendered, redirects not followed) and returns `{ status, url, html }`.
2. A `303` (or `401` / `403`) makes `fetch.mjs` throw an error with `code: "auth"`. The run ends with `auth: …`, `ctx.browser` turns that into `SiteError("auth")`, and the sync leaves the source as `needs_login` with `login_url` = `home_url` (`/members`, which takes the owner to the sign-in page). The articles the site already has stay.
3. Signed in, `extract.ts` parses the HTML with `ctx.html`: the list is `.posts .post`, each with `a.post-link` and `time[datetime]`; an article is `article.member-post` with `.post-title`, `.post-meta .author` and the paragraphs under `.post-body`.

One background run per page (the list, and one per new article); no page is rendered, the tab stays blank.

The job does not pause itself: it runs again on its schedule (`every_secs` 21600 for a source with a sign-in), fails as `auth` until the owner has signed in, and goes back to `ok` by itself afterwards.

## What goes wrong

- `auth: … asks to sign in (HTTP 303)`: the owner is not signed in, or the session ended (the test site forgets all sessions at `/__reset`). Not a fault of the program: the Goal tells the owner once, with the link.
- `the page has no ".posts .post"`: the markup changed. `/__markup?v=2` renames the classes (`.entries .entry`, `a.entry__title`, `article.entry-full`, `.entry-full__title`, `.entry-full__meta .by`, `.entry-full__text`): a repair. `/__markup?v=3` is another site (a table of links to `/members/read/<slug>`, text in `.sheet-line`): an upgrade.

## Running it without an Agent

`test/agent_browser_dv/README.md`, "Sources that need the browser": the stand-in `browser_stub.mjs` has a browser profile of its own, and `sign_in.mjs` plays the owner signing in.
