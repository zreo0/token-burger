import { describe, it, expect } from 'vitest';
import { matchModelPrice, calculateModelCost, calculateTotalCost } from '../pricing';
import type { PricingTable, ModelPrice, TokenBreakdown } from '../../types';

const testPrice: ModelPrice = {
    input_cost_per_token: 3.0 / 1_000_000,
    output_cost_per_token: 15.0 / 1_000_000,
    cache_read_input_token_cost: 0.30 / 1_000_000,
    cache_creation_input_token_cost: 3.75 / 1_000_000,
};

const pricing: PricingTable = {
    'claude-sonnet-4': testPrice,
};

describe('matchModelPrice', () => {
    it('精确匹配', () => {
        expect(matchModelPrice('claude-sonnet-4', pricing)).toBe(testPrice);
    });

    it('归一化匹配（去日期后缀）', () => {
        expect(matchModelPrice('claude-sonnet-4-20250514', pricing)).toBe(testPrice);
    });

    it('Provider 前缀匹配', () => {
        expect(matchModelPrice('anthropic/claude-sonnet-4', pricing)).toBe(testPrice);
    });

    it('未匹配返回 null', () => {
        expect(matchModelPrice('unknown-model', pricing)).toBeNull();
    });
});

describe('calculateModelCost', () => {
    it('应正确计算费用', () => {
        const breakdown: TokenBreakdown = {
            input: 1_000_000,
            output: 100_000,
            cache_read: 0,
            cache_create: 0,
            agent_cost: 0,
        };
        const cost = calculateModelCost(breakdown, testPrice);
        // 3.0 + 1.5 = 4.5
        expect(cost).toBeCloseTo(4.5, 2);
    });
});

describe('calculateTotalCost', () => {
    it('应汇总多模型费用', () => {
        const byModel: Record<string, TokenBreakdown> = {
            'claude-sonnet-4': { input: 1_000_000, output: 0, cache_read: 0, cache_create: 0, agent_cost: 0 },
        };
        const cost = calculateTotalCost(byModel, pricing);
        expect(cost).toBeCloseTo(3.0, 2);
    });

    it('未匹配模型不计费', () => {
        const byModel: Record<string, TokenBreakdown> = {
            'unknown': { input: 1_000_000, output: 0, cache_read: 0, cache_create: 0, agent_cost: 0 },
        };
        const cost = calculateTotalCost(byModel, pricing);
        expect(cost).toBe(0);
    });

    it('优先使用 agent 自带 cost', () => {
        const byModel: Record<string, TokenBreakdown> = {
            'gpt-4.1': { input: 1_000_000, output: 0, cache_read: 0, cache_create: 0, agent_cost: 1.23 },
        };
        const cost = calculateTotalCost(byModel, pricing);
        expect(cost).toBe(1.23);
    });
});
