import { describe, expect, it } from 'vitest';
import type { TFunction } from 'i18next';
import { agentLabel, kindClass, localizedTipSummary, localizedTipTitle, shouldAutoHide } from '../index';
import type { BehaviorTip } from '../../../types';

const translations: Record<string, string> = {
    'behaviorTip.title.permission_requested': '需要你确认',
    'behaviorTip.title.run_completed': '运行完成',
    'behaviorTip.summary.codexPermissionRequested': 'Codex 正在等待权限确认',
    'behaviorTip.summary.codexRunAbortedWithReason': 'Codex 停止了当前轮次：{{reason}}',
};

const t = ((key: string, options?: { defaultValue?: string; reason?: string }) => {
    const value = translations[key] ?? options?.defaultValue ?? key;
    return options?.reason ? value.replace('{{reason}}', options.reason) : value;
}) as TFunction;

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

    it('本地化已知标题和摘要', () => {
        const tip = makeTip();

        expect(localizedTipTitle(t, tip)).toBe('需要你确认');
        expect(localizedTipSummary(t, tip)).toBe('Codex 正在等待权限确认');
    });

    it('保留未知动态摘要，避免误翻', () => {
        const tip = makeTip({
            kind: 'tool_error',
            title: 'Tool error',
            summary: 'Custom tool failed',
        });

        expect(localizedTipSummary(t, tip)).toBe('Custom tool failed');
    });

    it('本地化带原因的 Codex 中断摘要', () => {
        const tip = makeTip({
            kind: 'run_aborted',
            title: 'Run stopped',
            summary: 'Codex stopped the current turn: user canceled',
        });

        expect(localizedTipSummary(t, tip)).toBe('Codex 停止了当前轮次：user canceled');
    });
});
