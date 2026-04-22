import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { AnimatePresence, motion } from 'framer-motion';
import type { AgentInfo, AppSettings, PlatformInfo } from '../../types';
import { getPlatformInfo } from '../../utils/platform';
import { BURGER_THEMES } from '../../components/Burger/themes';
import './index.css';

type Tab = 'general' | 'agents' | 'data';

function Settings() {
    const { t, i18n } = useTranslation();
    const [tab, setTab] = useState<Tab>('general');
    const [settings, setSettings] = useState<AppSettings | null>(null);
    const [agents, setAgents] = useState<AgentInfo[]>([]);
    const [platformInfo, setPlatformInfo] = useState<PlatformInfo | null>(null);
    const [confirmAction, setConfirmAction] = useState<string | null>(null);

    const loadSettings = useCallback(async () => {
        try {
            const s = await invoke<AppSettings>('get_settings');
            setSettings(s);
            if (s.language) {
                i18n.changeLanguage(s.language);
            }
        } catch {
            // 使用默认值
        }
    }, [i18n]);

    const loadAgents = useCallback(async () => {
        try {
            const list = await invoke<AgentInfo[]>('get_agent_list');
            setAgents(list);
        } catch {
            // 忽略
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

    const updateSetting = async (key: string, value: string) => {
        await invoke('update_settings', { key, value });
        loadSettings();
    };

    const handleToggleAgent = async (agentName: string, enabled: boolean) => {
        await invoke('toggle_agent', { agentName, enabled });
        loadAgents();
        loadSettings();
    };

    const handleClearData = async (all: boolean) => {
        try {
            const keepDays = all ? null : settings?.keep_days ?? 365;
            await invoke('clear_data', { keepDays });
            setConfirmAction(null);
        } catch {
            // 忽略
        }
    };

    return (
        <div className="settings-shell">
            <div className="settings-container">
                <div className="settings-header">
                    <div className="settings-tabs">
                        {(['general', 'agents', 'data'] as Tab[]).map((t_) => (
                            <button
                                key={t_}
                                type="button"
                                className={`settings-tab ${tab === t_ ? 'active' : ''}`}
                                onClick={() => {
                                    setTab(t_);
                                    setConfirmAction(null);
                                }}
                            >
                                {t(`settings.${t_}`)}
                            </button>
                        ))}
                    </div>
                </div>

                <div className="settings-content-wrapper">
                    <AnimatePresence mode="wait">
                        <motion.div
                            key={tab}
                            initial={{ opacity: 0, y: 4, filter: 'blur(2px)' }}
                            animate={{ opacity: 1, y: 0, filter: 'blur(0px)' }}
                            exit={{ opacity: 0, y: -4, filter: 'blur(2px)' }}
                            transition={{ duration: 0.15, ease: 'easeOut' }}
                            className="settings-content"
                        >
                            {tab === 'general' && settings && (
                                <div className="settings-group">
                                    <div className="setting-row">
                                        <span className="setting-label">{t('settings.language')}</span>
                                        <div className="select-wrapper">
                                            <select
                                                value={settings.language}
                                                onChange={(e) => {
                                                    updateSetting('language', e.target.value);
                                                    i18n.changeLanguage(e.target.value);
                                                }}
                                            >
                                                <option value="en">English</option>
                                                <option value="zh-CN">简体中文</option>
                                            </select>
                                        </div>
                                    </div>
                                    <div className="setting-divider" />
                                    <div className="setting-row">
                                        <span className="setting-label">{t('settings.colorTheme')}</span>
                                        <div className="theme-picker">
                                            {BURGER_THEMES.map((theme) => (
                                                <button
                                                    key={theme.id}
                                                    type="button"
                                                    className={`theme-option ${settings.color_theme === theme.id ? 'active' : ''}`}
                                                    onClick={() => updateSetting('color_theme', theme.id)}
                                                    title={t(theme.labelKey)}
                                                >
                                                    <span className="theme-swatches">
                                                        {Object.values(theme.colors).map((color, i) => (
                                                            <span key={i} className="theme-dot" style={{ backgroundColor: color }} />
                                                        ))}
                                                    </span>
                                                </button>
                                            ))}
                                        </div>
                                    </div>
                                    <div className="setting-divider" />
                                    <div className="setting-row">
                                        <span className="setting-label">{t('settings.watchMode')}</span>
                                        <div className="segmented-control">
                                            {['realtime', 'polling'].map((mode) => (
                                                <button
                                                    key={mode}
                                                    type="button"
                                                    className={`segment-btn ${settings.watch_mode === mode ? 'active' : ''}`}
                                                    onClick={() => updateSetting('watch_mode', mode)}
                                                >
                                                    {t(`settings.${mode}`)}
                                                </button>
                                            ))}
                                        </div>
                                    </div>
                                </div>
                            )}

                            {tab === 'agents' && settings && (
                                <div className="settings-group">
                                    {agents.map((agent, index) => (
                                        <div key={agent.name}>
                                            <div className={`setting-row agent-row ${!agent.available ? 'unavailable' : ''}`}>
                                                <div className="agent-info">
                                                    <div className="agent-name-row">
                                                        <span className="agent-name">{agent.name}</span>
                                                        <span className="agent-source-badge">{agent.source_type}</span>
                                                    </div>
                                                    <span className="agent-status">
                                                        {agent.available
                                                            ? t(agent.enabled ? 'settings.enabled' : 'settings.disabled')
                                                            : t('settings.notDetected')}
                                                    </span>
                                                </div>
                                                <label className="mac-toggle">
                                                    <input
                                                        type="checkbox"
                                                        checked={agent.enabled}
                                                        disabled={!agent.available}
                                                        onChange={() => handleToggleAgent(agent.name, !agent.enabled)}
                                                    />
                                                    <span className="mac-toggle-slider" />
                                                </label>
                                            </div>
                                            {index < agents.length - 1 && <div className="setting-divider" />}
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
                                                    min={1}
                                                    max={365}
                                                    value={settings.keep_days}
                                                    onChange={(e) => updateSetting('keep_days', e.target.value)}
                                                />
                                                <span className="suffix">{t('settings.days')}</span>
                                            </div>
                                        </div>
                                    </div>

                                    <div className="settings-group mt-4">
                                        {confirmAction ? (
                                            <div className="setting-row confirm-row">
                                                <span className="confirm-text">{t('settings.clearConfirm')}</span>
                                                <div className="action-buttons">
                                                    <button
                                                        className="mac-btn"
                                                        type="button"
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
                                            <div className="setting-row">
                                                <span className="setting-label">{t('settings.data')}</span>
                                                <div className="action-buttons">
                                                    <button
                                                        className="mac-btn"
                                                        type="button"
                                                        onClick={() => setConfirmAction('clearOld')}
                                                    >
                                                        {t('settings.clearOld')}
                                                    </button>
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
                        </motion.div>
                    </AnimatePresence>
                </div>

                <div className="settings-footer">
                    {platformInfo && (
                        <div className="platform-badge">
                            <span className="footer-label">{t('settings.platform')}</span>
                            <span className="footer-value">{platformInfo.display_name}</span>
                        </div>
                    )}

                    {import.meta.env.DEV && (
                        <div className="dev-mode-badge">DEV MODE</div>
                    )}
                </div>
            </div>
        </div>
    );
}

export default Settings;
