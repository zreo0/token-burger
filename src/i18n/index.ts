import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import en from './locales/en.json';
import zhCN from './locales/zh-CN.json';

i18n
    .use(initReactI18next)
    .init({
        resources: {
            en: { translation: en },
            'zh-CN': { translation: zhCN },
        },
        lng: navigator.language.startsWith('zh') ? 'zh-CN' : 'en',
        fallbackLng: 'en',
        interpolation: {
            escapeValue: false,
        },
    });

export default i18n;
