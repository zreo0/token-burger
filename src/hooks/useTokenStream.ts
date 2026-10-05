import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { TokenSummary, TimeRange } from '../types';

export function useTokenStream() {
    const [summary, setSummary] = useState<TokenSummary | null>(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);
    const [range, setRange] = useState<TimeRange>('today');
    const requestId = useRef(0);

    /**
     * 查询当前范围；后台更新保留已有内容，避免历史补录期间数字反复变成占位
     */
    const fetchSummary = useCallback(async (background: boolean) => {
        const id = ++requestId.current;
        try {
            if (!background) setLoading(true);
            const result = await invoke<TokenSummary>('get_token_summary', { range });
            if (id !== requestId.current) return;
            setSummary(result);
            setError(null);
        } catch (e) {
            if (id === requestId.current) setError(String(e));
        } finally {
            if (id === requestId.current) setLoading(false);
        }
    }, [range]);

    // 重新打开弹窗和手动刷新保留当前数据，仅在首次查询或范围切换时显示占位
    const refresh = useCallback(() => fetchSummary(true), [fetchSummary]);

    useEffect(() => {
        void fetchSummary(false);
        // 范围切换和卸载后，旧请求不得覆盖新范围的数据
        return () => { requestId.current += 1; };
    }, [fetchSummary]);

    useEffect(() => {
        const unlisten = listen<TokenSummary>('token-updated', (event) => {
            if (range === 'today') {
                // today 视图直接使用推送的汇总（已是最新）
                requestId.current += 1;
                setSummary(event.payload);
                setLoading(false);
                setError(null);
            } else {
                // 7d/30d 视图重新查询（当天新增 token 影响该范围总量）
                void fetchSummary(true);
            }
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, [range, fetchSummary]);

    return { summary, loading, error, refresh, range, setRange };
}
