import { describe, it, expect } from 'vitest';
import { formatTokenCount, formatCost } from '../format';

describe('formatTokenCount', () => {
    it('应返回原始数字（<1000）', () => {
        expect(formatTokenCount(0)).toBe('0');
        expect(formatTokenCount(999)).toBe('999');
    });

    it('应格式化为 K', () => {
        expect(formatTokenCount(1000)).toBe('1.0K');
        expect(formatTokenCount(1500)).toBe('1.5K');
    });

    it('应格式化为 M', () => {
        expect(formatTokenCount(1000000)).toBe('1.0M');
        expect(formatTokenCount(1500000)).toBe('1.5M');
    });

    it('应格式化为 B', () => {
        expect(formatTokenCount(1500000000)).toBe('1.5B');
    });
});

describe('formatCost', () => {
    it('应格式化为 $X.XX', () => {
        expect(formatCost(0)).toBe('$0.00');
        expect(formatCost(1.5)).toBe('$1.50');
        expect(formatCost(123.456)).toBe('$123.46');
    });

    it('极小值应显示 <$0.01', () => {
        expect(formatCost(0.001)).toBe('<$0.01');
    });
});
