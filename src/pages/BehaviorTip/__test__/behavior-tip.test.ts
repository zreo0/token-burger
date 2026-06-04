import { describe, expect, it } from 'vitest';
import { agentLabel, kindClass, shouldAutoHide } from '../index';
import type { BehaviorTip } from '../../../types';

function makeTip(overrides: Partial<BehaviorTip> = {}): BehaviorTip {
    return {
        key: 'tip-1',
        agent_name: 'codex',
        session_id: 'session-1',
        kind: 'permission_requested',
        timestamp: '2026-06-01T10:00:00Z',
        title: 'Permission needed',
        summary: 'Codex is waiting for permission',
        ...overrides,
    };
}

describe('BehaviorTip helpers', () => {
    it('映射 Agent 展示名', () => {
        expect(agentLabel('codex')).toBe('Codex');
        expect(agentLabel('opencode')).toBe('OpenCode');
        expect(agentLabel('custom-agent')).toBe('custom-agent');
    });

    it('按事件类型映射视觉状态', () => {
        expect(kindClass('permission_requested')).toBe('permission');
        expect(kindClass('run_completed')).toBe('success');
        expect(kindClass('run_aborted')).toBe('warning');
        expect(kindClass('tool_error')).toBe('warning');
    });

    it('只在存在正数自动隐藏时间时自动隐藏', () => {
        expect(shouldAutoHide(makeTip({ auto_hide_ms: 5000 }))).toBe(true);
        expect(shouldAutoHide(makeTip({ auto_hide_ms: 0 }))).toBe(false);
        expect(shouldAutoHide(makeTip())).toBe(false);
    });
});

