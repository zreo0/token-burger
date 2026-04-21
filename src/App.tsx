import { useEffect } from 'react';
import { HashRouter, Routes, Route } from 'react-router-dom';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { TokenProvider } from './context/TokenContext';
import Popup from './pages/Popup';
import Settings from './pages/Settings';
import i18n from './i18n';
import type { AppSettings } from './types';

function App() {
    useEffect(() => {
        let disposed = false;

        const syncLanguage = async () => {
            try {
                const settings = await invoke<AppSettings>('get_settings');

                if (!disposed && settings.language && i18n.language !== settings.language) {
                    await i18n.changeLanguage(settings.language);
                }
            } catch {
                // 忽略，保留默认语言
            }
        };

        syncLanguage();

        const unlisten = listen<string>('settings-language-changed', async (event) => {
            if (event.payload && i18n.language !== event.payload) {
                await i18n.changeLanguage(event.payload);
            }
        });

        return () => {
            disposed = true;
            unlisten.then((fn) => fn());
        };
    }, []);

    return (
        <TokenProvider>
            <HashRouter>
                <Routes>
                    <Route path="/" element={<Popup />} />
                    <Route path="/settings" element={<Settings />} />
                </Routes>
            </HashRouter>
        </TokenProvider>
    );
}

export default App;
