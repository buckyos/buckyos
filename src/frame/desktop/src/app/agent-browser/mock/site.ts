/* The personal site in the mock runtime. Its data is in the mock bucket of mock/reflow.ts, put there by whoever
 * plays the Agent: `window.__agentBrowserMockSeed.objects` before the App opens (the e2e specs seed the shared
 * samples of doc/opendan/protocol/fixtures/site/ this way), `window.__agentBrowserMock.site.put` after. The page is a
 * stand-in for the site's template: it draws the routes from `_data` and speaks the messages of Personal Site
 * Protocol §6.2, nothing more. */

/** What a spec (or a hand in the console) puts in place before the App's mock is loaded. */
export interface MockSeed {
  /** Objects of the user's reflow bucket: key → text. */
  objects?: Record<string, string>
  /** Agents (by id) that have read-only access. */
  readOnly?: string[]
  /** `browser.page_hint` by the host of the current tab. */
  hints?: Record<string, 'reading' | 'task' | 'unknown'>
}

export const mockSeed: MockSeed = (window as { __agentBrowserMockSeed?: MockSeed }).__agentBrowserMockSeed ?? {}

export const MOCK_SITE_PAGE = `<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Reading site</title>
<style>
  html { background: #fff; color: #1c1c1e; }
  html[data-theme='dark'] { background: #1c1c1e; color: #f2f2f3; }
  body { margin: 0 auto; max-width: 40rem; padding: 0 1rem 2rem; font: 16px/1.6 system-ui, sans-serif; }
  h1 { margin: 1rem 0 0.25rem; font-size: 1.35rem; line-height: 1.3; }
  h2 { margin: 1.25rem 0 0; font-size: 0.8rem; font-weight: 600; color: #6b6b70; }
  p { margin: 0.6rem 0; }
  a, button { font: inherit; }
  .meta { margin: 0; color: #6b6b70; font-size: 0.85rem; }
  .item { display: block; padding: 0.7rem 0; border-bottom: 1px solid #e3e3e6; color: inherit; text-decoration: none; }
  .item small { display: block; color: #6b6b70; line-height: 1.45; }
  .source { font-weight: 600; }
  button { margin: 1rem 0.5rem 0 0; padding: 0.45rem 0.9rem; border: 1px solid #c9c9ce; border-radius: 0.6rem; background: none; color: inherit; }
</style>
</head>
<body>
<main id="main"></main>
<script>
const mock = parent.__agentBrowserMock.site;
const tell = (message) => parent.postMessage(message, '*');
const json = (key) => { try { return JSON.parse(mock.read(key)); } catch { return null; } };
const el = (tag, props, ...children) => { const node = Object.assign(document.createElement(tag), props); node.append(...children); return node; };
const to = (route, props, ...children) => el('a', { href: '#', onclick: (event) => { event.preventDefault(); go(route); }, ...props }, ...children);
let route = window.__reflowView.query;
let draws = 0;
// Which sign-in of the frame this document came from, for the specs.
document.documentElement.dataset.opened = String(window.__reflowView.opened);
function draw() {
  const main = document.getElementById('main');
  const params = new URLSearchParams(route);
  const site = json('_data/site.json');
  main.replaceChildren();
  document.body.dataset.draws = String(++draws);
  document.body.dataset.route = route;
  if (!site) {
    main.append(el('p', { id: 'empty', textContent: 'No site yet' }));
    tell({ type: 'site.location', route, title: '' });
    return;
  }
  if (params.get('a')) {
    const article = json('_data/a/' + params.get('a') + '.json');
    if (!article) {
      main.append(el('h1', { id: 'title', textContent: 'Not collected' }));
      tell({ type: 'site.location', route, title: '' });
      return;
    }
    main.append(el('h1', { id: 'title', textContent: article.title.t }), el('p', { className: 'meta', textContent: article.byline }));
    for (const block of article.blocks) if (block.text) main.append(el(block.type === 'h2' || block.type === 'h3' ? 'h2' : 'p', { textContent: block.text.t }));
    for (const link of article.links) {
      const open = (event) => { event.preventDefault(); if (link.article) go('a=' + link.article); else tell({ type: 'site.open', url: link.url }); };
      main.append(el('a', { href: '#', className: 'item', onclick: open }, link.text.t));
    }
    main.append(el('button', { id: 'original', type: 'button', textContent: 'View original', onclick: () => tell({ type: 'site.open', url: article.url }) }));
    tell({ type: 'site.location', route, title: article.title.t, url: article.url, source: article.source });
    return;
  }
  const one = params.get('s');
  const sources = site.sources.filter((source) => !one || source.id === one);
  main.append(el('h1', { id: 'title', textContent: one ? (sources[0] ? sources[0].title : one) : site.title }));
  if (one) main.append(to('', { id: 'front', className: 'meta' }, 'All websites'));
  for (const source of sources) {
    const feed = json('_data/feed/' + source.id + '.json');
    if (!one) main.append(el('h2', {}, to('s=' + source.id, { className: 'source', id: 'source-' + source.id }, source.title)));
    if (!feed) main.append(el('p', { className: 'meta', textContent: source.state === 'failed' ? 'Could not be added' : 'Being prepared' }));
    for (const item of feed ? feed.items : []) main.append(to('a=' + item.id, { className: 'item', id: 'item-' + item.id }, item.title, el('small', { textContent: item.summary })));
  }
  main.append(el('button', { id: 'add', type: 'button', textContent: 'Add a website', onclick: () => tell({ type: 'site.add' }) }));
  if (one && sources[0]) main.append(el('button', { id: 'add-here', type: 'button', textContent: 'Add this website', onclick: () => tell({ type: 'site.add', url: sources[0].home_url }) }));
  tell(one ? { type: 'site.location', route, title: document.getElementById('title').textContent, source: one } : { type: 'site.location', route, title: site.title });
}
function go(next) {
  route = next;
  draw();
  scrollTo(0, 0);
}
addEventListener('message', (event) => {
  if (event.source !== parent || !event.data) return;
  if (event.data.type === 'site.navigate') go(String(event.data.route || ''));
  if (event.data.type === 'site.updated') draw();
  if (event.data.type === 'site.theme') document.documentElement.dataset.theme = event.data.theme;
});
// The specs let the page's session run out: it says so instead of loading itself again.
window.loseSession = () => tell({ type: 'site.session' });
draw();
tell({ type: 'site.ready' });
</script>
</body>
</html>
`
