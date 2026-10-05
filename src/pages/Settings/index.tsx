import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
import { check, Update } from '@tauri-apps/plugin-updater';
import { openUrl } from '@tauri-apps/plugin-opener';
import type {
    AccountUsageProviderInfo,
    AgentInfo,
    AppSettings,
    PlatformInfo,
    PricingRefreshResult,
} from '../../types';
import { getPlatformInfo } from '../../utils/platform';
import { BURGER_THEMES } from '../../components/Burger/themes';
import { useAccountUsageContext } from '../../context/AccountUsageContext';
import claudeCodeProviderIcon from '../../assets/provider-icons/claude-code.svg';
import openaiProviderIcon from '../../assets/provider-icons/openai.svg';
import githubCopilotProviderIcon from '../../assets/provider-icons/github-copilot.svg';
import opencodeProviderIcon from '../../assets/provider-icons/opencode.svg';
import './index.css';

type Tab = 'general' | 'agents' | 'alerts' | 'data' | 'usage' | 'about';

/** 各设置分类使用同一描边风格的图标 */
const TAB_ICON_PATHS: Record<Tab, string> = {
    general: 'M4 7h16M4 17h16M8 4v6M16 14v6',
    agents: 'M8 4H5a1 1 0 0 0-1 1v14a1 1 0 0 0 1 1h3m8-16h3a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1h-3M14 8l-4 8',
    alerts: 'M6 10a6 6 0 0 1 12 0v5l2 3H4l2-3v-5m4 11h4',
    data: 'M4 6c0-4 16-4 16 0s-16 4-16 0v12c0 4 16 4 16 0V6M4 12c0 4 16 4 16 0',
    usage: 'M4 5h16v14H4V5m0 5h16m-5 5h2',
    about: 'M12 8h.01M11 12h1v5m10-5a10 10 0 1 1-20 0 10 10 0 0 1 20 0',
};

// 账号用量 Provider 逐个开放，菜单栏展示仅对可计算百分比的 Provider 启用。
const VISIBLE_USAGE_PROVIDER_IDS = new Set(['codex', 'claude-code', 'github-copilot', 'opencode-go']);
const USAGE_PROVIDER_ICONS: Record<string, string> = {
    codex: openaiProviderIcon,
    'claude-code': claudeCodeProviderIcon,
    'github-copilot': githubCopilotProviderIcon,
    'opencode-go': opencodeProviderIcon,
};

type UpdateStatus =
    | { state: 'idle' }
    | { state: 'checking' }
    | { state: 'no-update' }
    | { state: 'update-available'; version: string; update: Update }
    | { state: 'downloading'; progress: number }
    | { state: 'ready-to-restart'; update: Update }
    | { state: 'error'; message: string };

type PricingReloadStatus =
    | { state: 'idle' }
    | { state: 'loading' }
    | { state: 'success'; modelCount: number }
    | { state: 'error' };

/**
 * 请求后端强制重新加载模型价格
 * 无参数，返回后端刷新结果 Promise
 */
export function requestPricingReload(): Promise<PricingRefreshResult> {
    return invoke<PricingRefreshResult>('reload_pricing');
}

function canShowProviderInMenuBar(provider: AccountUsageProviderInfo): boolean {
    return provider.capabilities.includes('account_quota');
}

/**
 * 根据账号配置返回简短的状态翻译键
 */
function usageProviderStatusKey(provider: AccountUsageProviderInfo): string {
    if (provider.enabled) return 'settings.enabled';
    if (provider.available) return 'usage.available';
    return 'settings.notDetected';
}

/**
 * 渲染分组设置与保存反馈，无参数，返回桌面设置界面
 */
function Settings() {
    const { t, i18n } = useTranslation();
    const {
        providers: usageProviders,
        isLoading: usageLoading,
        reload: reloadUsage,
        setEnabled: setUsageEnabled,
        setMenuBarVisible,
        saveCredential,
        clearCredential,
    } = useAccountUsageContext();
    const [tab, setTab] = useState<Tab>('general');
    const [loadError, setLoadError] = useState(false);
    const [saving, setSaving] = useState(false);
    const [agentsLoading, setAgentsLoading] = useState(true);
    const [agentsError, setAgentsError] = useState(false);
    const [actionError, setActionError] = useState(false);
    const [settings, setSettings] = useState<AppSettings | null>(null);
    const [agents, setAgents] = useState<AgentInfo[]>([]);
    const [platformInfo, setPlatformInfo] = useState<PlatformInfo | null>(null);
    const [confirmAction, setConfirmAction] = useState<string | null>(null);
    const [appVersion, setAppVersion] = useState('');
    const [updateStatus, setUpdateStatus] = useState<UpdateStatus>({ state: 'idle' });
    const [pricingReloadStatus, setPricingReloadStatus] = useState<PricingReloadStatus>({ state: 'idle' });
    const visibleUsageProviders = usageProviders.filter(provider => VISIBLE_USAGE_PROVIDER_IDS.has(provider.id));
    const isMac = platformInfo?.platform === 'macos';

    useEffect(() => {
        getVersion().then(setAppVersion).catch(() => {});
    }, []);

    /**
     * 读取设置并同步语言，无参数，结果写入组件状态
     */
    const loadSettings = useCallback(async () => {
        try {
            const s = await invoke<AppSettings>('get_settings');
            setSettings(s);
            setLoadError(false);
            if (s.language) {
                i18n.changeLanguage(s.language);
            }
        } catch {
            setLoadError(true);
        }
    }, [i18n]);

    /**
     * 读取本地 Agent 列表并区分加载、空列表和失败，无参数和返回值
     */
    const loadAgents = useCallback(async () => {
        setAgentsLoading(true);
        try {
            const list = await invoke<AgentInfo[]>('get_agent_list');
            setAgents(list);
            setAgentsError(false);
        } catch {
            setAgentsError(true);
        } finally {
            setAgentsLoading(false);
        }
    }, []);

    useEffect(() => {
        loadSettings();
        loadAgents();

        getPlatformInfo()
            .then(setPlatformInfo)
            .catch(() => {
                // 忽略
            });
    }, [loadSettings, loadAgents]);

    /**
     * 执行设置操作并提供忙碌与失败反馈，参数为异步操作，无返回值
     */
    const runSettingsAction = async (action: () => Promise<void>): Promise<void> => {
        setSaving(true);
        setActionError(false);
        try {
            await action();
        } catch {
            setActionError(true);
        } finally {
            setSaving(false);
        }
    };

    /**
     * 保存指定键值并重新读取设置，参数为设置键和值，无返回值
     */
    const updateSetting = async (key: string, value: string): Promise<void> => {
        await runSettingsAction(async () => {
            await invoke('update_settings', { key, value });
            await loadSettings();
        });
    };

    /**
     * 切换指定 Agent 并同步设置，参数为 Agent 名称和开关值，无返回值
     */
    const handleToggleAgent = async (agentName: string, enabled: boolean): Promise<void> => {
        await runSettingsAction(async () => {
            await invoke('toggle_agent', { agentName, enabled });
            await Promise.all([loadAgents(), loadSettings()]);
        });
    };

    /**
     * 执行已确认的数据清理，参数为是否清空全部，成功后收起确认区
     */
    const handleClearData = async (all: boolean): Promise<void> => {
        await runSettingsAction(async () => {
            const keepDays = all ? null : settings?.keep_days ?? 90;
            await invoke('clear_data', { keepDays });
            setConfirmAction(null);
        });
    };

    /**
     * 强制刷新模型价格并更新设置页反馈
     * 无参数和返回值
     */
    const handleReloadPricing = async (): Promise<void> => {
        setPricingReloadStatus({ state: 'loading' });
        try {
            const result = await requestPricingReload();
            setPricingReloadStatus({ state: 'success', modelCount: result.model_count });
        } catch {
            setPricingReloadStatus({ state: 'error' });
        }
    };

    /**
     * 检查应用更新并保留结果反馈，无参数，返回完成检查的 Promise
     */
    const handleCheckUpdate = async () => {
        setUpdateStatus({ state: 'checking' });
        try {
            const update = await check();
            if (update) {
                setUpdateStatus({ state: 'update-available', version: update.version, update });
            } else {
                setUpdateStatus({ state: 'no-update' });
            }
        } catch {
            // 获取更新失败时不展示具体错误细节，但需要和“已是最新版”区分开。
            setUpdateStatus({ state: 'error', message: t('settings.updateCheckFailed') });
        }
    };

    const handleDownloadUpdate = async (update: Update) => {
        setUpdateStatus({ state: 'downloading', progress: 0 });
        try {
            let totalLength = 0;
            let downloaded = 0;
            await update.downloadAndInstall((event) => {
                if (event.event === 'Started' && event.data.contentLength) {
                    totalLength = event.data.contentLength;
                } else if (event.event === 'Progress') {
                    downloaded += event.data.chunkLength;
                    const pct = totalLength > 0 ? Math.round((downloaded / totalLength) * 100) : 0;
                    setUpdateStatus({ state: 'downloading', progress: pct });
                } else if (event.event === 'Finished') {
                    setUpdateStatus({ state: 'ready-to-restart', update });
                }
            });
        } catch {
            setUpdateStatus({ state: 'error', message: t('common.error') });
        }
    };

    const handleRestart = async () => {
        await invoke('restart_app');
    };

    return (
        <div className="settings-shell">
            <div className="settings-container">
                <aside className="settings-sidebar">
                    <div className="settings-brand">
                        <span className="settings-brand-mark" aria-hidden="true"><i /><i /><i /><i /></span>
                        <span>TokenBurger</span>
                    </div>
                    <nav className="settings-tabs" aria-label={t('settings.title')}>
                        {(['general', 'agents', 'usage', 'alerts', 'data', 'about'] as Tab[]).map((item) => (
                            <button
                                key={item}
                                type="button"
                                className={`settings-tab ${tab === item ? 'active' : ''}`}
                                aria-current={tab === item ? 'page' : undefined}
                                onClick={() => {
                                    setTab(item);
                                    setConfirmAction(null);
                                    setActionError(false);
                                }}
                            >
                                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={TAB_ICON_PATHS[item]} /></svg>
                                {t(`settings.${item}`)}
                            </button>
                        ))}
                    </nav>
                    <div className="settings-footer">
                        {platformInfo && <span className="footer-value">{platformInfo.display_name}</span>}
                        {appVersion && <span className="footer-value">v{appVersion}</span>}
                        {import.meta.env.DEV && <span className="dev-mode-badge">DEV</span>}
                    </div>
                </aside>
                <main className="settings-main">
                    <header className="settings-header">
                        <div className="settings-title-row">
                            <h1 id="settings-page-title">{t(`settings.${tab}`)}</h1>
                            <span className="settings-save-status" role="status">{saving ? t('settings.working') : ''}</span>
                        </div>
                        <p>{t(`settings.descriptions.${tab}`)}</p>
                    </header>
                    <div className="settings-content-wrapper" key={tab}>
                        {actionError && <p className="settings-feedback error" role="alert">{t('settings.saveFailed')}</p>}
                        {loadError && <div className="settings-feedback error" role="alert">{t('settings.loadFailed')} <button className="mac-btn" type="button" onClick={loadSettings}>{t('common.retry')}</button></div>}
                        {!settings && !loadError && <p className="settings-feedback" role="status">{t('common.loading')}</p>}
                        <fieldset
                            disabled={saving}
                            className="settings-content"
                            aria-busy={saving}
                            aria-labelledby="settings-page-title"
                        >
                            {tab === 'general' && settings && (
                                <div className="settings-group">
                                    <div className="setting-row">
                                        <span className="setting-label">{t('settings.language')}</span>
                                        <div className="select-wrapper">
                                            <select
                                                aria-label={t('settings.language')}
                                                value={settings.language}
                                                onChange={(e) => {
                                                    updateSetting('language', e.target.value);
                                                }}
                                            >
                                                <option value="en">English</option>
                                                <option value="zh-CN">简体中文</option>
                                            </select>
                                        </div>
                                    </div>
                                    <div className="setting-row theme-setting-row">
                                        <div className="setting-copy">
                                            <span className="setting-label">{t('settings.colorTheme')}</span>
                                            <span className="setting-hint">{t('settings.themeHint')}</span>
                                        </div>
                                        <div className="theme-picker">
                                            {BURGER_THEMES.map((theme) => (
                                                <button
                                                    key={theme.id}
                                                    type="button"
                                                    className={`theme-option ${settings.color_theme === theme.id ? 'active' : ''}`}
                                                    onClick={() => updateSetting('color_theme', theme.id)}
                                                    title={t(theme.labelKey)}
                                                    aria-pressed={settings.color_theme === theme.id}
                                                >
                                                    <span className="theme-swatches">
                                                        {['output', 'cache_read', 'cache_create', 'input'].map((key, i) => (
                                                            // 小预览保持与主弹窗一致的食材顺序
                                                            <span key={i} className="theme-dot" style={{ backgroundColor: theme.colors[key as keyof typeof theme.colors] }} />
                                                        ))}
                                                    </span>
                                                    <span className="theme-name">{t(theme.labelKey)}<svg className="theme-selected-icon" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" aria-hidden="true"><path d="m3 8 3 3 7-7" /></svg></span>
                                                </button>
                                            ))}
                                        </div>
                                    </div>
                                    <div className="setting-row">
                                        <span className="setting-label">{t('settings.watchMode')}</span>
                                        <div className="settings-segments" aria-label={t('settings.watchMode')}>
                                            {['realtime', 'polling'].map((mode) => (
                                                <button
                                                    key={mode}
                                                    type="button"
                                                    className={`segment-btn ${settings.watch_mode === mode ? 'active' : ''}`}
                                                    aria-pressed={settings.watch_mode === mode}
                                                    onClick={() => updateSetting('watch_mode', mode)}
                                                >
                                                    {t(`settings.${mode}`)}
                                                </button>
                                            ))}
                                        </div>
                                    </div>
                                </div>
                            )}

                            {tab === 'alerts' && settings && (
                                <div className="settings-group">
                                    <div className="setting-row">
                                        <div className="setting-copy">
                                            <span className="setting-label">{t('settings.runAlerts')}</span>
                                            <span className="setting-hint">{t('settings.runAlertsHint')}</span>
                                        </div>
                                        <label className="mac-toggle">
                                            <input
                                                type="checkbox"
                                                aria-label={t('settings.runAlerts')}
                                                checked={settings.behavior_tips_enabled}
                                                onChange={() => updateSetting('behavior_tips_enabled', String(!settings.behavior_tips_enabled))}
                                            />
                                            <span className="mac-toggle-slider" />
                                        </label>
                                    </div>
                                </div>
                            )}

                            {tab === 'agents' && settings && (
                                <div className="settings-group">
                                    {agentsError && <div className="settings-feedback error" role="alert">{t('settings.agentsLoadFailed')} <button className="mac-btn" type="button" onClick={loadAgents}>{t('common.retry')}</button></div>}
                                    {agentsLoading && agents.length === 0 && <p className="settings-empty" role="status">{t('common.loading')}</p>}
                                    {!agentsLoading && !agentsError && agents.length === 0 && <p className="settings-empty">{t('settings.noAgents')}</p>}
                                    {agents.map((agent) => (
                                        <div key={agent.name}>
                                            <div className={`setting-row agent-row ${!agent.available ? 'unavailable' : ''}`}>
                                                <div className="agent-info">
                                                    <div className="agent-name-row">
                                                        <span className="agent-name">{agent.name}</span>
                                                        <span className="agent-source-badge">{agent.source_type}</span>
                                                    </div>
                                                    <span className={`agent-status ${agent.available && agent.enabled ? 'enabled' : ''}`}>
                                                        {agent.available
                                                            ? t(agent.enabled ? 'settings.enabled' : 'settings.disabled')
                                                            : t('settings.notDetected')}
                                                    </span>
                                                </div>
                                                <label className="mac-toggle">
                                                    <input
                                                        type="checkbox"
                                                        aria-label={agent.name}
                                                        checked={agent.enabled}
                                                        disabled={!agent.available}
                                                        onChange={() => handleToggleAgent(agent.name, !agent.enabled)}
                                                    />
                                                    <span className="mac-toggle-slider" />
                                                </label>
                                            </div>
                                        </div>
                                    ))}
                                </div>
                            )}

                            {tab === 'data' && settings && (
                                <>
                                    <div className="settings-group">
                                        <div className="setting-row">
                                            <span className="setting-label">{t('settings.keepDays')}</span>
                                            <div className="mac-number-input">
                                                <input
                                                    type="number"
                                                    aria-label={t('settings.keepDays')}
                                                    min={1}
                                                    max={365}
                                                    defaultValue={settings.keep_days}
                                                    onBlur={(e) => {
                                                        if (e.currentTarget.reportValidity() && Number(e.target.value) !== settings.keep_days) {
                                                            void updateSetting('keep_days', e.target.value);
                                                        }
                                                    }}
                                                    onKeyDown={(e) => { if (e.key === 'Enter') e.currentTarget.blur(); }}
                                                    required
                                                />
                                                <span className="suffix">{t('settings.days')}</span>
                                            </div>
                                        </div>
                                        <div className="setting-row">
                                            <div className="setting-copy">
                                                <span className="setting-label">{t('settings.modelPricing')}</span>
                                                <span className="setting-hint">
                                                    {pricingReloadStatus.state === 'success'
                                                        ? t('settings.pricingRefreshSuccess', { count: pricingReloadStatus.modelCount })
                                                        : pricingReloadStatus.state === 'error'
                                                            ? t('settings.pricingRefreshFailed')
                                                            : t('settings.modelPricingHint')}
                                                </span>
                                            </div>
                                            <button
                                                className="mac-btn"
                                                type="button"
                                                disabled={pricingReloadStatus.state === 'loading'}
                                                onClick={handleReloadPricing}
                                            >
                                                {pricingReloadStatus.state === 'loading'
                                                    ? t('settings.refreshingPricing')
                                                    : t('settings.refreshPricing')}
                                            </button>
                                        </div>
                                    </div>

                                    <div className="settings-group">
                                        {confirmAction ? (
                                            <div className="setting-row confirm-row">
                                                <div className="setting-copy">
                                                    <strong className="setting-label">{t(`settings.${confirmAction}`)}</strong>
                                                    <span className="confirm-text">{t('settings.clearConfirm')}</span>
                                                </div>
                                                <div className="action-buttons">
                                                    <button
                                                        className="mac-btn"
                                                        type="button"
                                                        autoFocus
                                                        onClick={() => setConfirmAction(null)}
                                                    >
                                                        {t('settings.cancel')}
                                                    </button>
                                                    <button
                                                        className="mac-btn danger-text"
                                                        type="button"
                                                        onClick={() => handleClearData(confirmAction === 'clearAll')}
                                                    >
                                                        {t('settings.confirm')}
                                                    </button>
                                                </div>
                                            </div>
                                        ) : (
                                            <div className="cleanup-options">
                                                <div className="setting-row">
                                                    <div className="setting-copy"><span className="setting-label">{t('settings.clearOld')}</span><span className="setting-hint">{t('settings.clearOldHint', { days: settings.keep_days })}</span></div>
                                                    <button
                                                        className="mac-btn"
                                                        type="button"
                                                        onClick={() => setConfirmAction('clearOld')}
                                                    >
                                                        {t('settings.clearOld')}
                                                    </button>
                                                </div>
                                                <div className="setting-row">
                                                    <div className="setting-copy"><span className="setting-label">{t('settings.clearAll')}</span><span className="setting-hint">{t('settings.clearAllHint')}</span></div>
                                                    <button
                                                        className="mac-btn danger-text"
                                                        type="button"
                                                        onClick={() => setConfirmAction('clearAll')}
                                                    >
                                                        {t('settings.clearAll')}
                                                    </button>
                                                </div>
                                            </div>
                                        )}
                                    </div>
                                </>
                            )}
                            {tab === 'usage' && (
                                <div className="settings-group usage-provider-compact-list">
                                    {usageLoading && visibleUsageProviders.length === 0 && <p className="settings-empty" role="status">{t('common.loading')}</p>}
                                    {!usageLoading && visibleUsageProviders.length === 0 && <div className="settings-feedback">{t('settings.noUsageProviders')} <button type="button" className="mac-btn" onClick={reloadUsage}>{t('common.retry')}</button></div>}
                                    <div className="usage-provider-compact-header">
                                        <span>{t('usage.provider', 'Provider')}</span>
                                        <span>{t('usage.accountUsageShort', 'Usage')}</span>
                                        <span>{t('usage.menuBarShort', 'Menu bar')}</span>
                                    </div>
                                    {visibleUsageProviders.map((provider) => {
                                        const menuBarAvailable = isMac && provider.enabled && canShowProviderInMenuBar(provider);
                                        const menuBarHint = !isMac
                                            ? t('usage.menuBarMacOnly', 'macOS only')
                                            : provider.enabled
                                                ? t('usage.menuBarDisplayHint', 'Show the provider icon and usage percent')
                                                : t('usage.enableProviderFirst', 'Enable account usage first');
                                        const accountUsageHint = provider.available
                                            ? t('usage.refreshEvery', { seconds: provider.refresh_interval_secs })
                                            : provider.credential_requirements?.length > 0
                                                ? t('usage.requiresCredential', 'Requires credential')
                                                : t('usage.notDetected', 'Auth file or credential not detected');
                                        const shouldShowCredentialForm = provider.credential_requirements?.length > 0
                                            && (!provider.available || provider.id === 'claude-code');
                                        const statusKey = usageProviderStatusKey(provider);

                                        return (
                                            <div key={provider.id}>
                                                <div className={`usage-provider-compact-row ${provider.enabled ? 'enabled' : ''}`}>
                                                    <div className="usage-provider-identity">
                                                        <span className={`usage-provider-avatar provider-${provider.id}`} aria-hidden="true">
                                                            {USAGE_PROVIDER_ICONS[provider.id] ? (
                                                                <img src={USAGE_PROVIDER_ICONS[provider.id]} alt="" />
                                                            ) : (
                                                                provider.display_name.slice(0, 1)
                                                            )}
                                                        </span>
                                                        <div className="usage-provider-title-stack">
                                                            <div className="usage-provider-title-line">
                                                                <span className="usage-provider-compact-name">{provider.display_name}</span>
                                                                <span className={`usage-provider-status-badge ${provider.enabled ? 'enabled' : ''}`}>
                                                                    {t(statusKey)}
                                                                </span>
                                                            </div>
                                                            {!provider.available && (
                                                                <span className="usage-provider-compact-hint">{accountUsageHint}</span>
                                                            )}
                                                        </div>
                                                    </div>
                                                    <label className="usage-provider-switch-cell" title={accountUsageHint}>
                                                        <input
                                                            type="checkbox"
                                                            checked={provider.enabled}
                                                            onChange={(e) => { const enabled = e.target.checked; void runSettingsAction(() => setUsageEnabled(provider.id, enabled)); }}
                                                            aria-label={`${provider.display_name} ${t('usage.accountUsageShort', 'Usage')}`}
                                                        />
                                                        <span className="usage-provider-checkmark" />
                                                    </label>
                                                    <label className={`usage-provider-switch-cell ${!menuBarAvailable ? 'disabled' : ''}`} title={menuBarHint}>
                                                        <input
                                                            type="checkbox"
                                                            checked={provider.enabled && provider.show_in_menu_bar}
                                                            disabled={!menuBarAvailable}
                                                            onChange={(e) => { const visible = e.target.checked; void runSettingsAction(() => setMenuBarVisible(provider.id, visible)); }}
                                                            aria-label={`${provider.display_name} ${t('usage.menuBarShort', 'Menu bar')}`}
                                                        />
                                                        <span className="usage-provider-checkmark" />
                                                    </label>
                                                </div>
                                                {shouldShowCredentialForm && (
                                                    <details className="credential-disclosure" open={!provider.available}>
                                                        <summary>{t('settings.credentials')}<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true"><path d="m6 4 4 4-4 4" /></svg></summary>
                                                        <form
                                                            className="usage-credential-inline-form"
                                                            onSubmit={(e) => {
                                                                e.preventDefault();
                                                                const formData = new FormData(e.currentTarget);
                                                                const secretRequirement = provider.credential_requirements.find(req => req.secret);
                                                                if (!secretRequirement) return;
                                                                const accountRequirement = provider.credential_requirements.find(req => !req.secret);
                                                                const accountKey = accountRequirement
                                                                    ? String(formData.get(accountRequirement.key) ?? '').trim()
                                                                    : undefined;
                                                                const secret = String(formData.get(secretRequirement.key) ?? '').trim();

                                                                // 多字段 Provider 将公开账号标识复用为 account_key，密钥仍只进入系统凭据存储
                                                                const form = e.currentTarget;
                                                                void runSettingsAction(async () => {
                                                                    await saveCredential(
                                                                        provider.id,
                                                                        secretRequirement.key,
                                                                        secret,
                                                                        accountKey || secretRequirement.label,
                                                                        accountKey,
                                                                    );
                                                                    form.reset();
                                                                });
                                                            }}
                                                        >
                                                            <div className="usage-credential-inline-fields">
                                                                {provider.credential_requirements.map(req => (
                                                                    <label key={req.key} className="usage-credential-inline-field">
                                                                        <span>{req.label}</span>
                                                                        <input
                                                                            name={req.key}
                                                                            type={req.secret ? 'password' : 'text'}
                                                                            placeholder={req.description}
                                                                            required={req.required}
                                                                            autoComplete="off"
                                                                            spellCheck={false}
                                                                        />
                                                                    </label>
                                                                ))}
                                                            </div>
                                                            <button type="submit" className="mac-btn">{t('usage.saveCredential', 'Save Credential')}</button>
                                                            <button type="button" className="mac-btn danger-text" onClick={() => { void runSettingsAction(() => clearCredential(provider.id)); }}>{t('usage.clearCredential', 'Clear')}</button>
                                                        </form>
                                                    </details>
                                                )}
                                            </div>
                                        );
                                    })}
                                </div>
                            )}
                            {tab === 'about' && (
                                <>
                                    <div className="settings-group">
                                        <div className="setting-row">
                                            <span className="setting-label">{t('settings.version')}</span>
                                            <span className="setting-value">{appVersion}</span>
                                        </div>
                                        <div className="setting-row">
                                            <span className="setting-label">{t('settings.github')}</span>
                                            <button
                                                type="button"
                                                className="about-link"
                                                onClick={() => openUrl('https://github.com/zreo0/token-burger')}
                                            >
                                                zreo0/token-burger
                                            </button>
                                        </div>
                                        {(updateStatus.state === 'idle' || updateStatus.state === 'checking' || updateStatus.state === 'no-update' || updateStatus.state === 'error') && (
                                            <div className="setting-row">
                                                <div className="setting-copy">
                                                    <span className="setting-label">{t('settings.appUpdates')}</span>
                                                    <span className={`about-status-text ${updateStatus.state === 'error' ? 'about-error-text' : updateStatus.state === 'no-update' ? 'about-success' : ''}`} role="status">
                                                        {updateStatus.state === 'no-update' ? t('settings.upToDate') : updateStatus.state === 'error' ? updateStatus.message : ''}
                                                    </span>
                                                </div>
                                                <button type="button" className="mac-btn" onClick={handleCheckUpdate} disabled={updateStatus.state === 'checking'}>
                                                    {t(updateStatus.state === 'checking' ? 'settings.checking' : updateStatus.state === 'error' ? 'common.retry' : 'settings.checkUpdate')}
                                                </button>
                                            </div>
                                        )}
                                        {updateStatus.state === 'update-available' && (
                                            <div className="setting-row about-update-row about-update-available">
                                                <span className="about-status-text">
                                                    {t('settings.newVersion', { version: updateStatus.version })}
                                                </span>
                                                <div className="action-buttons">
                                                    <button
                                                        type="button"
                                                        className="mac-btn"
                                                        onClick={() => setUpdateStatus({ state: 'idle' })}
                                                    >
                                                        {t('settings.later')}
                                                    </button>
                                                    <button
                                                        type="button"
                                                        className="mac-btn about-primary-btn"
                                                        onClick={() => handleDownloadUpdate(updateStatus.update)}
                                                    >
                                                        {t('settings.download')}
                                                    </button>
                                                </div>
                                            </div>
                                        )}
                                        {updateStatus.state === 'downloading' && (
                                            <div className="setting-row about-update-row about-downloading">
                                                <span className="about-status-text">
                                                    {t('settings.downloading', { progress: updateStatus.progress })}
                                                </span>
                                                <div className="about-progress-bar" role="progressbar" aria-label={t('settings.download')} aria-valuemin={0} aria-valuemax={100} aria-valuenow={updateStatus.progress}>
                                                    <div
                                                        className="about-progress-fill"
                                                        style={{ transform: `scaleX(${updateStatus.progress / 100})` }}
                                                    />
                                                </div>
                                            </div>
                                        )}
                                        {updateStatus.state === 'ready-to-restart' && (
                                            <div className="setting-row about-update-row about-update-available">
                                                <span className="about-status-text">{t('settings.readyToRestart')}</span>
                                                <button
                                                    type="button"
                                                    className="mac-btn about-primary-btn"
                                                    onClick={handleRestart}
                                                >
                                                    {t('settings.restart')}
                                                </button>
                                            </div>
                                        )}

                                    </div>
                                </>
                            )}
                        </fieldset>
                    </div>
                </main>
            </div>
        </div>
    );
}

export default Settings;
