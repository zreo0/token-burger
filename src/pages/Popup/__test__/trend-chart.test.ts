import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import TrendChart, { getChangePercent, totalTokens } from '../trend-chart';

describe('trend comparisons', () => {
    it('历史不足与零基数不显示误导性增幅', () => {
        expect(getChangePercent(100, 0, true)).toBeNull();
        expect(getChangePercent(100, 50, false)).toBeNull();
        expect(getChangePercent(118, 100, true)).toBe('+18%');
        expect(getChangePercent(0, 100, true)).toBe('-100%');
    });

    it('趋势包含缓存和输出，但不将费用算作 Token', () => {
        expect(totalTokens({ model: { input: 10, output: 20, cache_read: 30, cache_create: 40, agent_cost: 3 } })).toBe(100);
        expect(totalTokens({})).toBe(0);
    });
});

vi.mock('react-i18next', () => ({
    useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));

describe('calendar trend labels', () => {
    it.each(['7d', '30d'] as const)('%s 显示自然日而非滚动时刻，并标明今天未结束', (range) => {
        const start = new Date(2026, 9, 5).getTime() / 1000;
        const trend = {
            buckets: [{ start, end: start + 86400, by_model: { model: { input: 10, output: 0, cache_read: 0, cache_create: 0, agent_cost: 0 } } }],
            previous_by_model: {}, comparison_available: false,
        };
        const markup = renderToStaticMarkup(React.createElement(TrendChart, { trend, pricing: {}, range, loading: false, error: false, onRetry: vi.fn() }));
        expect(markup).toContain('10/5 00:00 – 24:00');
        expect(markup).toContain('popup.todayIncomplete');
        expect(markup).not.toContain('popup.now');
    });
});
