import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { 
    AccountUsageSnapshot, 
    AccountUsageProviderInfo,
    AccountUsageProviderState
} from '../types';

export function mergeAccountUsageSnapshots(
    current: AccountUsageSnapshot[],
    incoming: AccountUsageSnapshot[],
): AccountUsageSnapshot[] {
    if (incoming.length === 0) return current;

    const incomingProviders = new Set(incoming.map(snapshot => snapshot.provider_id));
    return [
        ...current.filter(snapshot => !incomingProviders.has(snapshot.provider_id)),
        ...incoming,
    ];
}

export function filterSnapshotsByEnabledProviders(
    snapshots: AccountUsageSnapshot[],
    providers: AccountUsageProviderInfo[],
): AccountUsageSnapshot[] {
    const enabledProviderIds = new Set(providers.filter(provider => provider.enabled).map(provider => provider.id));

    return snapshots.filter(snapshot => enabledProviderIds.has(snapshot.provider_id));
}

async function refreshEnabledProviders() {
    return await invoke<AccountUsageSnapshot[]>('refresh_account_usage_all');
}

export function useAccountUsage() {
    const [snapshots, setSnapshots] = useState<AccountUsageSnapshot[]>([]);
    const [providers, setProviders] = useState<AccountUsageProviderInfo[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [refreshing, setRefreshing] = useState(false);
    const [refreshingProviders, setRefreshingProviders] = useState<Record<string, boolean>>({});
    const [providerErrors, setProviderErrors] = useState<Record<string, string>>({});
    const [authorizingProviders, setAuthorizingProviders] = useState<Record<string, boolean>>({});

    const loadData = useCallback(async () => {
        setIsLoading(true);
        try {
            const [fetchedProviders, fetchedSnapshots] = await Promise.all([
                invoke<AccountUsageProviderInfo[]>('list_account_usage_providers'),
                invoke<AccountUsageSnapshot[]>('get_account_usage_snapshots')
            ]);
            setProviders(fetchedProviders);
            setSnapshots(filterSnapshotsByEnabledProviders(fetchedSnapshots, fetchedProviders));
        } catch (err) {
            console.error('Failed to load account usage data:', err);
        } finally {
            setIsLoading(false);
        }
    }, []);

    useEffect(() => {
        loadData();

        let unlistenSnapshots: (() => void) | undefined;
        let unlistenProviders: (() => void) | undefined;
        const setupListener = async () => {
            unlistenSnapshots = await listen<AccountUsageSnapshot[]>('account-usage-updated', (event) => {
                setSnapshots(prev => mergeAccountUsageSnapshots(prev, event.payload));
                // 后台或其他窗口恢复成功后清理旧的操作错误，仅 available 变化不足以证明已恢复
                setProviderErrors(prev => {
                    const next = { ...prev };
                    for (const snapshot of event.payload) {
                        if (snapshot.status === 'ok' && !snapshot.stale && !snapshot.error) {
                            delete next[snapshot.provider_id];
                        }
                    }
                    return next;
                });
            });
            unlistenProviders = await listen<AccountUsageProviderInfo[]>('account-usage-providers-updated', (event) => {
                setProviders(event.payload);
                setSnapshots(prev => filterSnapshotsByEnabledProviders(prev, event.payload));
            });
        };

        setupListener();

        return () => {
            if (unlistenSnapshots) unlistenSnapshots();
            if (unlistenProviders) unlistenProviders();
        };
    }, [loadData]);

    const refreshAll = async () => {
        setRefreshing(true);
        try {
            const newSnapshots = await refreshEnabledProviders();
            setSnapshots(prev => mergeAccountUsageSnapshots(prev, newSnapshots));
            setProviderErrors({});
        } catch (err) {
            console.error('Failed to refresh all:', err);
            setProviderErrors(Object.fromEntries(providers.filter(provider => provider.enabled).map(provider => [provider.id, String(err)])));
        } finally {
            setRefreshing(false);
        }
    };

    const refreshProvider = async (providerId: string) => {
        setRefreshingProviders(prev => ({ ...prev, [providerId]: true }));
        try {
            const newSnapshots = await invoke<AccountUsageSnapshot[]>('refresh_account_usage_provider', { providerId });
            setSnapshots(prev => mergeAccountUsageSnapshots(prev, newSnapshots));
            setProviderErrors(prev => {
                const next = { ...prev };
                delete next[providerId];
                return next;
            });
        } catch (err) {
            console.error(`Failed to refresh provider ${providerId}:`, err);
            setProviderErrors(prev => ({ ...prev, [providerId]: String(err) }));
        } finally {
            setRefreshingProviders(prev => ({ ...prev, [providerId]: false }));
        }
    };

    const saveCredential = async (
        providerId: string,
        secretKind: string,
        secret: string,
        label?: string,
        accountKey?: string,
    ) => {
        await invoke('save_account_usage_credential', {
            request: {
                provider_id: providerId,
                account_key: accountKey,
                secret_kind: secretKind,
                secret,
                label,
            },
        });
        // Reload providers to update available/enabled state
        const fetchedProviders = await invoke<AccountUsageProviderInfo[]>('list_account_usage_providers');
        setProviders(fetchedProviders);
        await refreshProvider(providerId);
    };

    const clearCredential = async (providerId: string) => {
        await invoke('clear_account_usage_credential', { providerId });
        const fetchedProviders = await invoke<AccountUsageProviderInfo[]>('list_account_usage_providers');
        setProviders(fetchedProviders);
        await refreshProvider(providerId);
    };

    /**
     * 仅响应用户点击导入本机登录，参数为服务标识，更新授权与额度状态
     */
    const authorizeProvider = async (providerId: string) => {
        setAuthorizingProviders(prev => ({ ...prev, [providerId]: true }));
        setProviderErrors(prev => ({ ...prev, [providerId]: '' }));
        try {
            const newSnapshots = await invoke<AccountUsageSnapshot[]>('authorize_account_usage_provider', { providerId });
            const fetchedProviders = await invoke<AccountUsageProviderInfo[]>('list_account_usage_providers');
            setProviders(fetchedProviders);
            setSnapshots(prev => mergeAccountUsageSnapshots(prev, newSnapshots));
        } catch (error) {
            setProviderErrors(prev => ({ ...prev, [providerId]: String(error) }));
        } finally {
            setAuthorizingProviders(prev => ({ ...prev, [providerId]: false }));
        }
    };

    const setEnabled = async (providerId: string, enabled: boolean) => {
        await invoke('set_account_usage_provider_enabled', { request: { provider_id: providerId, enabled } });
        const fetchedProviders = await invoke<AccountUsageProviderInfo[]>('list_account_usage_providers');
        setProviders(fetchedProviders);
        if (enabled) {
            const provider = fetchedProviders.find(provider => provider.id === providerId);
            if (provider?.capabilities.includes('local_credential_import') && !provider.available) {
                await authorizeProvider(providerId);
            } else {
                await refreshProvider(providerId);
            }
        } else {
            setSnapshots(prev => prev.filter(snapshot => snapshot.provider_id !== providerId));
            setProviderErrors(prev => {
                const next = { ...prev };
                delete next[providerId];
                return next;
            });
        }
    };

    const setMenuBarVisible = async (providerId: string, showInMenuBar: boolean) => {
        await invoke('set_account_usage_provider_menu_bar_visible', {
            request: { provider_id: providerId, show_in_menu_bar: showInMenuBar },
        });
        const fetchedProviders = await invoke<AccountUsageProviderInfo[]>('list_account_usage_providers');
        setProviders(fetchedProviders);
    };

    const getProviderState = async (providerId: string): Promise<AccountUsageProviderState> => {
        return await invoke<AccountUsageProviderState>('get_account_usage_provider_state', { providerId });
    };

    return {
        snapshots,
        providers,
        isLoading,
        refreshing,
        refreshingProviders,
        providerErrors,
        authorizingProviders,
        authorizeProvider,
        refreshAll,
        refreshProvider,
        saveCredential,
        clearCredential,
        setEnabled,
        setMenuBarVisible,
        getProviderState,
        reload: loadData
    };
}
