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

    const refresh = useCallback(async () => {
        const id = ++requestId.current;
        try {
            setLoading(true);
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

    useEffect(() => {
        void refresh();
        // 范围切换和卸载后，旧请求不得覆盖新范围的数据
        return () => { requestId.current += 1; };
    }, [refresh]);

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
                refresh();
            }
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, [range, refresh]);

    return { summary, loading, error, refresh, range, setRange };
}
