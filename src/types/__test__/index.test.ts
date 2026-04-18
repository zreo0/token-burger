import { describe, it, expect } from 'vitest';
import type { TokenLog } from '../index';

// 类型守卫：校验对象是否符合 TokenLog 结构
function isTokenLog(obj: unknown): obj is TokenLog {
    if (typeof obj !== 'object' || obj === null) return false;
    const o = obj as Record<string, unknown>;
    return (
        (o.id === undefined || typeof o.id === 'number') &&
        typeof o.agent_name === 'string' &&
        typeof o.provider === 'string' &&
        typeof o.model_id === 'string' &&
        ['input', 'cache_create', 'cache_read', 'output'].includes(o.token_type as string) &&
        typeof o.token_count === 'number' &&
        typeof o.is_error === 'boolean' &&
        typeof o.timestamp === 'string'
    );
}

describe('TokenLog 类型守卫', () => {
    it('合法的 TokenLog 对象应返回 true（无 id）', () => {
        const log: TokenLog = {
            agent_name: 'claude-code',
            provider: 'Anthropic',
            model_id: 'claude-3-7-sonnet',
            token_type: 'input',
            token_count: 1024,
            is_error: false,
            timestamp: '2026-04-18T10:00:00',
        };
        expect(isTokenLog(log)).toBe(true);
    });

    it('包含可选字段的 TokenLog 应返回 true（含 id）', () => {
        const log: TokenLog = {
            id: 2,
            agent_name: 'codex',
            provider: 'OpenAI',
            model_id: 'codex-mini',
            token_type: 'output',
            token_count: 512,
            session_id: 'sess-001',
            request_id: 'req-001',
            latency_ms: 350,
            is_error: false,
            metadata: '{"tool":"read"}',
            timestamp: '2026-04-18T11:00:00',
        };
        expect(isTokenLog(log)).toBe(true);
    });

    it('缺少必要字段应返回 false', () => {
        expect(isTokenLog({ id: 1 })).toBe(false);
        expect(isTokenLog({})).toBe(false);
    });

    it('非法 token_type 应返回 false', () => {
        const bad = {
            id: 1,
            agent_name: 'test',
            provider: 'test',
            model_id: 'test',
            token_type: 'invalid_type',
            token_count: 0,
            is_error: false,
            timestamp: '2026-04-18',
        };
        expect(isTokenLog(bad)).toBe(false);
    });

    it('null 和非对象应返回 false', () => {
        expect(isTokenLog(null)).toBe(false);
        expect(isTokenLog(undefined)).toBe(false);
        expect(isTokenLog('string')).toBe(false);
        expect(isTokenLog(42)).toBe(false);
    });
});
