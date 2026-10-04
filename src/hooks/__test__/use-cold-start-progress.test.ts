import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { listen, type Event } from '@tauri-apps/api/event';
import { subscribeColdStartProgress } from '../useColdStartProgress';
import type { ColdStartProgress } from '../../types';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

const scanning: ColdStartProgress = {
    live: true, revision: 1, phase: 'recent', agent: 'codex', files_checked: 42,
    errors: 0, total: 3, completed: 1, done: false,
};

describe('cold start progress subscription', () => {
    beforeEach(() => vi.resetAllMocks());

    it('打开页面恢复已有进度，并在完成事件后拒绝迟到的旧快照', async () => {
        let handler: (event: Event<ColdStartProgress>) => void = () => {};
        const unlisten = vi.fn();
        vi.mocked(listen).mockImplementation(async (_name, callback) => {
            handler = callback;
            return unlisten;
        });
        const complete = { ...scanning, revision: 3, phase: 'complete' as const, done: true };
        vi.mocked(invoke).mockImplementation(async () => {
            handler({ payload: complete } as Event<ColdStartProgress>);
            return scanning;
        });
        const update = vi.fn();
        const error = vi.fn();
        const cleanup = await subscribeColdStartProgress(update, error);
        expect(update).toHaveBeenCalledExactlyOnceWith(complete);
        expect(error).not.toHaveBeenCalled();
        cleanup();
        expect(unlisten).toHaveBeenCalledOnce();
    });

    it('没有新事件时使用查询快照，查询失败时反馈错误并保留监听', async () => {
        const unlisten = vi.fn();
        vi.mocked(listen).mockResolvedValue(unlisten);
        vi.mocked(invoke).mockResolvedValue(scanning);
        const update = vi.fn();
        const error = vi.fn();
        (await subscribeColdStartProgress(update, error))();
        expect(update).toHaveBeenCalledWith(scanning);
        vi.mocked(invoke).mockRejectedValue(new Error('offline'));
        (await subscribeColdStartProgress(update, error))();
        expect(error).toHaveBeenCalledOnce();
        expect(unlisten).toHaveBeenCalledTimes(2);
    });
});
