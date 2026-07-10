import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { requestPricingReload } from '../index';

vi.mock('@tauri-apps/api/core', () => ({
    invoke: vi.fn(),
}));

describe('requestPricingReload', () => {
    beforeEach(() => {
        vi.mocked(invoke).mockReset();
    });

    it('调用 reload_pricing 并返回刷新结果', async () => {
        vi.mocked(invoke).mockResolvedValue({ updated: true, model_count: 42 });

        await expect(requestPricingReload()).resolves.toEqual({
            updated: true,
            model_count: 42,
        });
        expect(invoke).toHaveBeenCalledWith('reload_pricing');
    });

    it('将刷新错误交给设置页显示失败状态', async () => {
        vi.mocked(invoke).mockRejectedValue(new Error('network'));

        await expect(requestPricingReload()).rejects.toThrow('network');
    });
});
