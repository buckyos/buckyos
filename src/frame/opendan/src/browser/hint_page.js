// Evaluated once in the tab the user looks at (`browser.page_hint`): counts
// what tells a page to read from a page to do something on. It reads the
// document and changes nothing in it.
(() => {
  const fields = [...document.querySelectorAll('input, select, textarea')].filter(
    (field) => !(field.localName === 'input' && ['hidden', 'submit', 'button', 'search'].includes(field.type)) && field.checkVisibility({ visibilityProperty: true }),
  );
  const ARTICLES = ['Article', 'NewsArticle', 'BlogPosting'];
  const isArticle = (node) => {
    if (Array.isArray(node)) return node.some(isArticle);
    if (!node || typeof node !== 'object') return false;
    return [node['@type']].flat().some((type) => ARTICLES.includes(type)) || isArticle(node['@graph']);
  };
  const described = (script) => {
    try {
      return isArticle(JSON.parse(script.textContent));
    } catch (_) {
      return false;
    }
  };
  const chars = (text) => (text ?? '').replace(/\s+/g, '').length;
  return {
    url: location.href,
    title: document.title,
    password_fields: fields.filter((field) => field.localName === 'input' && field.type === 'password').length,
    form_fields: fields.length,
    article_elements: document.querySelectorAll('article').length,
    og_article: (document.querySelector('meta[property="og:type"]')?.content ?? '').trim().toLowerCase() === 'article',
    ld_article: [...document.querySelectorAll('script[type="application/ld+json"]')].some(described),
    text_chars: chars(document.body?.innerText),
    link_chars: [...document.querySelectorAll('a')].reduce((sum, link) => sum + chars(link.innerText), 0),
  };
})()
