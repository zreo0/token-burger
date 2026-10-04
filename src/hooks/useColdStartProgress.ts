import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { ColdStartProgress } from '../types';

/**
 * 先订阅再查询进度，以修订号排除过期快照，返回取消订阅函数
 */
export async function subscribeColdStartProgress(
    onProgress: (progress: ColdStartProgress) => void,
    onError: () => void,
): Promise<UnlistenFn> {
    let revision = -1;
    /**
     * 接收进度，仅将未过期的快照交给页面，无返回值
     */
    const accept = (progress: ColdStartProgress): void => {
        if (progress.revision < revision) return;
        revision = progress.revision;
        onProgress(progress);
    };
    const unlisten = await listen<ColdStartProgress>('cold-start-progress', event => accept(event.payload));
    try {
        accept(await invoke<ColdStartProgress>('get_cold_start_progress'));
    } catch {
        if (revision < 0) onError();
    }
    return unlisten;
}

/**
 * 恢复并持续监听冷启动状态，无参数，返回进度、错误与重试操作
 */
export function useColdStartProgress() {
    const [progress, setProgress] = useState<ColdStartProgress | null>(null);
    const [error, setError] = useState(false);
    const [attempt, setAttempt] = useState(0);
    useEffect(() => {
        let disposed = false;
        setError(false);
        const subscription = subscribeColdStartProgress(value => {
            if (!disposed) {
                setProgress(value);
                setError(false);
            }
        }, () => { if (!disposed) setError(true); });
        subscription.catch(() => { if (!disposed) setError(true); });
        return () => {
            disposed = true;
            subscription.then(unlisten => unlisten()).catch(() => {});
        };
    }, [attempt]);
    return { progress, error, retry: () => setAttempt(value => value + 1) };
}
