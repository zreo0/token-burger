import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { resolve } from 'path';

// https://v2.tauri.app/start/frontend/vite/
export default defineConfig({
    plugins: [
        react(),
        tailwindcss(),
    ],
    resolve: {
        alias: {
            '@': resolve(__dirname, 'src'),
        },
    },
    clearScreen: false,
    server: {
        port: 1420,
        strictPort: true,
        host: '127.0.0.1',
        watch: {
            ignored: ['**/src-tauri/**'],
        },
    },
});
