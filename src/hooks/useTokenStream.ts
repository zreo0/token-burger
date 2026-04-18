import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { TokenSummary, TimeRange } from '../types';

export function useTokenStream() {
    const [summary, setSummary] = useState<TokenSummary | null>(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);
    const [range, setRange] = useState<TimeRange>('today');

    const refresh = useCallback(async () => {
        try {
            setLoading(true);
            const result = await invoke<TokenSummary>('get_token_summary', { range });
            setSummary(result);
            setError(null);
        } catch (e) {
            setError(String(e));
        } finally {
            setLoading(false);
        }
    }, [range]);

    useEffect(() => {
        refresh();
    }, [refresh]);

    useEffect(() => {
        const unlisten = listen<TokenSummary>('token-updated', (event) => {
            if (range === 'today') {
                // today 视图直接使用推送的汇总（已是最新）
                setSummary(event.payload);
            } else {
                // 7d/30d 视图重新查询（当天新增 token 影响该范围总量）
                refresh();
            }
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, [range, refresh]);

    return { summary, loading, error, refresh, range, setRange };
}
