import i18next from 'i18next';
import HttpBackend from 'i18next-http-backend';

i18next
  .use(HttpBackend)
  .init({
    lng: 'zh', 
    fallbackLng: 'en', 
    backend: {
      loadPath: './assets/{{lng}}.json' 
    },
    ns: ['common'], 
    defaultNS: 'common'
  }).then(() => {
    console.log("i18n init");
  });



function updateElementAndShadowRoots(root: Document | Element | ShadowRoot) {
    root.querySelectorAll('[data-i18n]').forEach(element => {
        const key = element.getAttribute('data-i18n');
        const options = element.getAttribute('data-i18n-options');
        //console.log(key, options);
        if (key?.startsWith('[html]')) {
            const actualKey = key.replace('[html]', '');
            element.innerHTML = i18next.t(actualKey, JSON.parse(options || '{}'));
        } else {
            element.textContent = i18next.t(key, JSON.parse(options || '{}'));
        }
    });


    root.querySelectorAll('*').forEach(element => {
        if (element.shadowRoot) {
            updateElementAndShadowRoots(element.shadowRoot);
        }
    });
}
export {i18next, updateElementAndShadowRoots};