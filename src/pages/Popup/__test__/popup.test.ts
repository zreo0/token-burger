import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import { listen } from '@tauri-apps/api/event';
import { getPopupWindowHeight, listenForPricingUpdates, Popup } from '../index';

vi.mock('react-i18next', () => ({
    useTranslation: () => ({
        t: (key: string) => key,
    }),
}));

vi.mock('@tauri-apps/api/core', () => ({
    invoke: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({
    listen: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock('../../../context/TokenContext', () => ({
    useToken: () => ({
        summary: {
            input: 10,
            cache_create: 0,
            cache_read: 0,
            output: 5,
            total: 15,
            agent_cost: 0,
            by_agent: {},
            by_model: {},
        },
        loading: false,
        error: null,
        refresh: vi.fn(),
        range: 'today',
        setRange: vi.fn(),
    }),
}));

vi.mock('../../../context/AccountUsageContext', () => ({
    useAccountUsageContext: () => ({
        snapshots: [],
        providers: [],
        reload: vi.fn(),
    }),
}));

vi.mock('../../../utils/platform', () => ({
    getPlatformInfo: vi.fn(() => Promise.resolve({ platform: 'macos', display_name: 'macOS' })),
}));

vi.mock('../../../components/Burger', () => ({
    default: () => React.createElement('div', { className: 'burger-mock' }, 'Burger'),
}));

vi.mock('../../../components/AccountUsageCard', () => ({
    default: () => null,
}));

describe('getPopupWindowHeight', () => {
    it('无账号时也按内容调整高度', () => {
        expect(getPopupWindowHeight(false, 640)).toBe(642);
        expect(getPopupWindowHeight(false, 0)).toBe(600);
    });

    it('有账号用量内容时按内容动态增高并限制最大值', () => {
        expect(getPopupWindowHeight(true, 520)).toBe(600);
        expect(getPopupWindowHeight(true, 620.2)).toBe(623);
        expect(getPopupWindowHeight(true, 900)).toBe(860);
    });
});

describe('Popup rendering', () => {
    it('不再在 Popup 内承担冷启动扫描状态展示', () => {
        const originalConsoleError = console.error;
        const consoleError = vi.spyOn(console, 'error').mockImplementation((message?: unknown, ...args: unknown[]) => {
            if (typeof message === 'string' && message.includes('useLayoutEffect does nothing')) {
                return;
            }
            originalConsoleError(message, ...args);
        });

        try {
            const markup = renderToStaticMarkup(React.createElement(Popup));

            expect(markup).not.toContain('cold-start-light');
            expect(markup).not.toContain('popup.coldStart');
        } finally {
            consoleError.mockRestore();
        }
    });
});

describe('pricing updates', () => {
    it('监听 pricing-updated 并触发价格重载', async () => {
        const onUpdate = vi.fn();
        const mockedListen = vi.mocked(listen);
        mockedListen.mockClear();

        await listenForPricingUpdates(onUpdate);

        expect(mockedListen).toHaveBeenCalledWith('pricing-updated', expect.any(Function));
        const handler = mockedListen.mock.calls[0][1];
        handler({} as never);
        expect(onUpdate).toHaveBeenCalledOnce();
    });
});
