import { describe, expect, it } from 'vitest';
import { getTopModels } from '../index';

describe('getTopModels', () => {
    it('按总 token 数排序并只返回前两个模型', () => {
        const topModels = getTopModels({
            alpha: {
                input: 120,
                cache_create: 40,
                cache_read: 10,
                output: 30,
                agent_cost: 0,
            },
            beta: {
                input: 80,
                cache_create: 60,
                cache_read: 50,
                output: 20,
                agent_cost: 0,
            },
            gamma: {
                input: 30,
                cache_create: 0,
                cache_read: 0,
                output: 5,
                agent_cost: 0,
            },
        });

        expect(topModels).toHaveLength(2);
        expect(topModels[0][0]).toBe('beta');
        expect(topModels[1][0]).toBe('alpha');
    });

    it('在无模型数据时返回空数组', () => {
        expect(getTopModels(undefined)).toEqual([]);
        expect(getTopModels(null)).toEqual([]);
    });
});
