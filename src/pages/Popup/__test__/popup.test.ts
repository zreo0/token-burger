import { describe, expect, it } from 'vitest';
import { getPopupWindowHeight, getTopModels } from '../index';

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

describe('getPopupWindowHeight', () => {
    it('无账号用量内容时保持默认高度', () => {
        expect(getPopupWindowHeight(false, 640)).toBe(540);
        expect(getPopupWindowHeight(false, 0)).toBe(540);
    });

    it('有账号用量内容时按内容动态增高并限制最大值', () => {
        expect(getPopupWindowHeight(true, 520)).toBe(540);
        expect(getPopupWindowHeight(true, 620.2)).toBe(623);
        expect(getPopupWindowHeight(true, 900)).toBe(680);
    });
});
