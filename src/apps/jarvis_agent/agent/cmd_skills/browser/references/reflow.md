# Reflow a page

Run `reflow match <url>` to get the site, page ID, reading preference and current `view`. Each command returns a `summary` and structured `detail`. The user's library survives Agent removal and can be shared by that user's Agents. The app shows saved results even when the browser is stopped. It chooses a matching package with a result, then a fallback. If a package lacks a result the app starts a refresh. If nothing is available or refresh fails, the app asks in the browser conversation.

#### One address or a failed refresh

Open the original URL with `agent-browser open <url>` and read its `snapshot`. For structured extraction use a run script. Keep only the content requested by the user's preference. Write a self-contained HTML file with site text escaped. Include original absolute image URLs only when requested. The frame on `https://reflow.<zone>/` carries the user's sign-in. Never copy site markup, scripts, styles, event attributes or `javascript:` URLs. Allow only HTTP(S) links. Load no external scripts, styles or fonts.

```html
<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Old site: a headline</title>
<style>
  body { margin: 0 auto; max-width: 40rem; padding: 1rem; font: 17px/1.7 system-ui, sans-serif; }
  img { max-width: 100%; }
</style>
</head>
<body>
<h1>Old site: a headline</h1>
<p>2026-11-03</p>
<p>The text of the page, with &amp;, &lt; and &gt; written as entities.</p>
<p><a href="https://www.example.com/news/2">Next page</a></p>
<script>
document.addEventListener('click', (event) => {
  const link = event.target.closest('a[href]');
  if (!link) return;
  event.preventDefault();
  parent.postMessage({ type: 'reflow.open', url: link.href }, '*');
});
parent.postMessage({ type: 'reflow.ready' }, '*');
</script>
</body>
</html>
```

Run `reflow result put <url> fallback.html`, then `reflow match <url>`. Check `fallback: made …` and `view` before reporting success. Only the user's session opens the frame directly.

#### A recurring site

Read `$OPENDAN_AGENT_ROOT/cmd_skills/browser/references/site-package.md` to build or update a package. For an existing one, start with `reflow get <site> <dir>` into a new/empty directory. If a site changes, give the waiting user a fallback first. Change the manifest preference and code together when requested. Periodic refresh belongs to a goal. Only use `reflow rm <site>` when asked: this deletes the package, preference, results and fallbacks.

#### Errors

| Reason | Action |
| --- | --- |
| `permission_denied` | Read-only Agents may use `match`, `ls`, `get`. Keep the files under `$OPENDAN_AGENT_ROOT`, give their Zone path (`/home/<owner>/agents/<agent_id>/…`) and ask the user to import them in Agent Browser. App-initiated refresh can save results. |
| `invalid` | Fix each entry in `detail.problems`. |
| `no_library` | Report the missing Zone library to the owner or administrator. |
| `no_browser` / `reflow_no_package` | Refresh on the private runtime. When no package matches, add one or save a fallback. |
| `refresh_failed` | Check the summary and run `try.mjs` from `$OPENDAN_AGENT_ROOT/cmd_skills/browser/references/preview.md` on the failing address. |
| Exit 4 | Refresh is still running after `--timeout`. Check `reflow match <url>` later. |

See `reflow --help` and the repository's `doc/opendan/protocol/Agent Browser Protocol.md` §5.
