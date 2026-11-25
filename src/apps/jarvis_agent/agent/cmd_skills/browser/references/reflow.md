# A one-off reflow page

For one address the owner wants tidied once ("this page only"): a single HTML file that stands by itself, kept in the owner's reflow library and shown in Agent Browser whether or not the browser is running. A website the owner keeps reading is not done this way: it becomes a source of their reading site.

1. `reflow match <url>` gives the site, the page and what is kept for the address: `fallback: none`, or `fallback: made …` with the `view` that shows it. Each command returns a `summary` and structured `detail`.
2. Read the original: `agent-browser open <url>` and `agent-browser snapshot`; a run script for a page that needs steps or structured extraction.
3. Write `page.html`: what the owner's `Note` asks for, otherwise title, date, author and the text in one column for a phone.
4. `reflow result put <url> page.html`, then `reflow match <url>`: report success only when it says `fallback: made …` and names the `view`. A later put for the same address replaces the page.

The page holds text of a third party: write it as text, with `&`, `<` and `>` as entities. Never copy the site's markup, scripts, styles, event attributes or `javascript:` addresses. Links are http(s) only. No script, style or font from another host; the original's pictures by their absolute address, and only when the owner wants them.

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

The page is shown in a frame of the app: `reflow.ready` says it is drawn, `reflow.open` hands a link to the app, which opens it. Only the owner's own session opens `https://reflow.<zone>/`: check your page with `reflow match`, not by opening it.

`reflow ls` lists the sites that have such pages. `reflow rm <site>` removes everything kept for a site: only when the owner asks.

| Result | Meaning and action |
| --- | --- |
| `permission_denied` | This Agent is read-only: `rm` and `result put` are refused. Keep the page under `$OPENDAN_AGENT_ROOT`, give its Zone path (`/home/<owner>/agents/<agent_id>/…`) and say that it could not be kept. |
| `invalid` | Not an http(s) address, or not a `.html` file: the summary says which. |
| `no_library` | This Zone has no reflow library: report it to the owner or the administrator. |
| `io_failed` (exit 5) | The local file could not be read. |

Syntax and exit codes: `reflow --help`.
