import { describe, expect, it } from 'vitest';
import { filterSnapshotsByEnabledProviders, getAccountUsageRefreshIntervalMs, mergeAccountUsageSnapshots } from '../useAccountUsage';
import type { AccountUsageProviderInfo, AccountUsageSnapshot } from '../../types';

function snapshot(providerId: string, accountKey = 'default'): AccountUsageSnapshot {
    return {
        provider_id: providerId,
        account_key: accountKey,
        status: 'ok',
        source: 'official_api',
        confidence: 'high',
        observed_at: '2026-05-13T00:00:00Z',
        stale: false,
        metrics: [],
    };
}

function provider(id: string, enabled: boolean): AccountUsageProviderInfo {
    return {
        id,
        display_name: id,
        enabled,
        available: true,
        source: 'auth_file',
        confidence: 'high',
        capabilities: ['account_usage'],
        credential_requirements: [],
        experimental: false,
        default_refresh_interval_secs: 300,
        refresh_interval_secs: 300,
    };
}

describe('mergeAccountUsageSnapshots', () => {
    it('合并单 Provider 刷新结果并保留其他 Provider 快照', () => {
        const merged = mergeAccountUsageSnapshots(
            [snapshot('codex', 'old'), snapshot('github-copilot')],
            [snapshot('codex', 'new')],
        );

        expect(merged.map(item => `${item.provider_id}:${item.account_key}`)).toEqual([
            'github-copilot:default',
            'codex:new',
        ]);
    });
});

describe('getAccountUsageRefreshIntervalMs', () => {
    it('使用已启用 Provider 的最短配置刷新间隔', () => {
        expect(getAccountUsageRefreshIntervalMs([
            provider('codex', true),
            { ...provider('github-copilot', true), refresh_interval_secs: 120 },
            { ...provider('cursor', false), refresh_interval_secs: 30 },
        ])).toBe(120000);
    });

    it('无启用 Provider 时不设置自动刷新', () => {
        expect(getAccountUsageRefreshIntervalMs([provider('codex', false)])).toBeNull();
    });
});

describe('filterSnapshotsByEnabledProviders', () => {
    it('过滤已关闭 Provider 的历史快照', () => {
        const filtered = filterSnapshotsByEnabledProviders(
            [snapshot('codex'), snapshot('github-copilot')],
            [provider('codex', false), provider('github-copilot', true)],
        );

        expect(filtered.map(item => item.provider_id)).toEqual(['github-copilot']);
    });
});
