import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import Burger from '..';
import {
    BREAD_HEIGHT,
    CACHE_MAX_HEIGHT,
    CACHE_MIN_HEIGHT,
    getCacheLayerHeight,
    getLayerSpringConfig,
} from '../BurgerLayer';

vi.mock('react-i18next', () => ({
    useTranslation: () => ({
        t: (key: string) => {
            const dict: Record<string, string> = {
                'popup.output': 'Output',
                'popup.cache_read': 'Cache Read',
                'popup.cache_create': 'Cache Write',
                'popup.input': 'Input',
            };

            return dict[key] ?? key;
        },
    }),
}));

vi.mock('framer-motion', async () => {
    const ReactModule = await import('react');

    return {
        motion: {
            div: ({ children, layout, transition, ...props }: React.HTMLAttributes<HTMLDivElement> & { layout?: unknown; transition?: unknown }) => ReactModule.createElement('div', props, children),
        },
        LayoutGroup: ({ children }: { children: React.ReactNode }) => ReactModule.createElement(ReactModule.Fragment, null, children),
        AnimatePresence: ({ children }: { children: React.ReactNode }) => ReactModule.createElement(ReactModule.Fragment, null, children),
        useSpring: (value: number) => ({
            get: () => value,
            set: () => {},
            on: () => () => {},
        }),
        useTransform: (
            source: { get?: () => number } | number,
            inputOrTransformer: ((latest: number) => number) | number[],
            output?: number[],
        ) => {
            const latest = typeof source === 'number' ? source : source.get?.() ?? 0;

            if (typeof inputOrTransformer === 'function') {
                return inputOrTransformer(latest);
            }

            return output?.[0] ?? latest;
        },
    };
});

describe('BurgerLayer helpers', () => {
    it('对缓存层高度应用非线性上下限', () => {
        expect(getCacheLayerHeight(0, 1000)).toBe(CACHE_MIN_HEIGHT);
        expect(getCacheLayerHeight(1000, 1000)).toBe(CACHE_MAX_HEIGHT);
        expect(getCacheLayerHeight(50, 1000)).toBeGreaterThan(CACHE_MIN_HEIGHT);
        expect(getCacheLayerHeight(50, 1000)).toBeLessThan(CACHE_MAX_HEIGHT);
        expect(BREAD_HEIGHT).toBeGreaterThan(CACHE_MIN_HEIGHT);
    });

    it('为 today 与范围切换返回不同弹簧配置', () => {
        const today = getLayerSpringConfig('today');
        const week = getLayerSpringConfig('7d');

        expect(today.stiffness).toBeLessThan(week.stiffness);
        expect(today.damping).toBeLessThan(week.damping);
    });
});

describe('Burger rendering', () => {
    it('零数据时仍渲染四层汉堡与层内数字', () => {
        const markup = renderToStaticMarkup(
            <Burger
                range="today"
                summary={{
                    input: 0,
                    cache_create: 0,
                    cache_read: 0,
                    output: 0,
                    total: 0,
                    agent_cost: 0,
                    by_agent: {},
                    by_model: {},
                }}
            />
        );

        expect(markup).toContain('Output');
        expect(markup).toContain('Cache Read');
        expect(markup).toContain('Cache Write');
        expect(markup).toContain('Input');
        expect(markup.match(/>0</g)?.length).toBe(4);
    });
});
