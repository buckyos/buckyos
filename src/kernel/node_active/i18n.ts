import i18next from 'i18next';
import HttpBackend from 'i18next-http-backend';

i18next
  .use(HttpBackend)
  .init({
    lng: 'en', 
    fallbackLng: 'en', // 降级语言
    backend: {
      loadPath: '/{{lng}}.json' 
    },
    ns: ['common'], 
    defaultNS: 'common'
  }).then(() => {
    console.log("i18n init");
  });

export default i18next;