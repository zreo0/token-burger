import { createElement } from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { filterSnapshotsByEnabledProviders, mergeAccountUsageSnapshots, useAccountUsage } from '../useAccountUsage';
import type { AccountUsageProviderInfo, AccountUsageSnapshot } from '../../types';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

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
        show_in_menu_bar: false,
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

describe('filterSnapshotsByEnabledProviders', () => {
    it('过滤已关闭 Provider 的历史快照', () => {
        const filtered = filterSnapshotsByEnabledProviders(
            [snapshot('codex'), snapshot('github-copilot')],
            [provider('codex', false), provider('github-copilot', true)],
        );

        expect(filtered.map(item => item.provider_id)).toEqual(['github-copilot']);
    });
});

describe('本机授权交互', () => {
    let usage: ReturnType<typeof useAccountUsage>;
    let renderer: ReactTestRenderer;
    let cursor: AccountUsageProviderInfo;
    const listeners = new Map<string, (payload: unknown) => void>();
    const authorize = vi.fn<() => Promise<AccountUsageSnapshot[]>>();

    /**
     * 使用真实 React 生命周期运行 Hook，暴露状态供交互断言
     */
    function Harness() {
        usage = useAccountUsage();
        return null;
    }

    beforeEach(async () => {
        vi.mocked(invoke).mockReset();
        vi.mocked(listen).mockReset();
        listeners.clear();
        cursor = { ...provider('cursor', false), available: false, capabilities: ['local_credential_import'] };
        authorize.mockReset().mockImplementation(async () => {
            cursor = { ...cursor, available: true };
            return [snapshot('cursor')];
        });
        vi.mocked(listen).mockImplementation(async (name, handler) => {
            listeners.set(String(name), payload => handler({ event: String(name), id: 1, payload }));
            return () => { listeners.delete(String(name)); };
        });
        vi.mocked(invoke).mockImplementation(async (command, args) => {
            if (command === 'list_account_usage_providers') return [cursor];
            if (command === 'get_account_usage_snapshots') return [];
            if (command === 'set_account_usage_provider_enabled') {
                cursor = { ...cursor, enabled: (args as { request: { enabled: boolean } }).request.enabled };
                return cursor;
            }
            if (command === 'authorize_account_usage_provider') return authorize();
            if (command === 'refresh_account_usage_provider') return [snapshot('cursor')];
            throw new Error(`未预期的命令: ${command}`);
        });
        await act(async () => { renderer = create(createElement(Harness)); });
    });

    afterEach(() => { act(() => { renderer.unmount(); }); });

    it('挂载不授权，首次开启只授权一次，已有授权时仅刷新', async () => {
        expect(authorize).not.toHaveBeenCalled();
        await act(async () => { await usage.setEnabled('cursor', true); });
        expect(authorize).toHaveBeenCalledTimes(1);
        expect(usage.providers[0].available).toBe(true);
        expect(usage.snapshots[0].provider_id).toBe('cursor');
        await act(async () => { await usage.setEnabled('cursor', false); });
        expect(usage.snapshots).toEqual([]);
        await act(async () => { await usage.setEnabled('cursor', true); });
        expect(authorize).toHaveBeenCalledTimes(1);
        expect(invoke).toHaveBeenCalledWith('refresh_account_usage_provider', { providerId: 'cursor' });
    });

    it('用户授权期间标记忙碌，拒绝授权后保留明确错误并结束等待', async () => {
        let reject!: (error: Error) => void;
        authorize.mockImplementation(() => new Promise((_resolve, fail) => { reject = fail; }));
        let pending!: Promise<void>;
        act(() => { pending = usage.authorizeProvider('cursor'); });
        expect(usage.authorizingProviders.cursor).toBe(true);
        await act(async () => { reject(new Error('未获得读取权限')); await pending; });
        expect(usage.authorizingProviders.cursor).toBe(false);
        expect(usage.providerErrors.cursor).toContain('未获得读取权限');
        expect(usage.providers[0].available).toBe(false);
    });

    it('跨窗口成功刷新清除对应旧错误，单纯 available 或失败快照不能清除', async () => {
        authorize.mockRejectedValue(new Error('旧授权错误'));
        await act(async () => { await usage.authorizeProvider('cursor'); await usage.authorizeProvider('other'); });
        act(() => { listeners.get('account-usage-providers-updated')?.([{ ...cursor, available: true }]); });
        expect(usage.providerErrors.cursor).toContain('旧授权错误');
        act(() => { listeners.get('account-usage-updated')?.([{ ...snapshot('cursor'), status: 'stale', stale: true, error: { code: 'network', message: '离线' } }]); });
        expect(usage.providerErrors.cursor).toContain('旧授权错误');
        act(() => { listeners.get('account-usage-updated')?.([snapshot('cursor')]); });
        expect(usage.providerErrors.cursor).toBeUndefined();
        expect(usage.providerErrors.other).toContain('旧授权错误');
    });
});
