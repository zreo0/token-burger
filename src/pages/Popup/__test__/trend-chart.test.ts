import { describe, expect, it } from 'vitest';
import { getChangePercent, totalTokens } from '../trend-chart';

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
