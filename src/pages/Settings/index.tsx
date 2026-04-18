import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { AnimatePresence, motion } from 'framer-motion';
import type { AgentInfo, AppSettings } from '../../types';
import './index.css';

type Tab = 'general' | 'agents' | 'data';

function Settings() {
    const { t, i18n } = useTranslation();
    const [tab, setTab] = useState<Tab>('general');
    const [settings, setSettings] = useState<AppSettings | null>(null);
    const [agents, setAgents] = useState<AgentInfo[]>([]);
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
        <div className="settings-container">
            <div className="settings-tabs">
                {(['general', 'agents', 'data'] as Tab[]).map((t_) => (
                    <button
                        key={t_}
                        className={`settings-tab ${tab === t_ ? 'active' : ''}`}
                        onClick={() => setTab(t_)}
                    >
                        {t(`settings.${t_}`)}
                    </button>
                ))}
            </div>

            <AnimatePresence mode="wait">
                <motion.div
                    key={tab}
                    initial={{ opacity: 0, y: 8 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -8 }}
                    transition={{ duration: 0.15 }}
                    className="settings-content"
                >
                    {tab === 'general' && settings && (
                        <div className="settings-section">
                            <div className="setting-row">
                                <label>{t('settings.language')}</label>
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
                            <div className="setting-row">
                                <label>{t('settings.watchMode')}</label>
                                <div className="segmented-control-sm">
                                    {['realtime', 'polling'].map((mode) => (
                                        <button
                                            key={mode}
                                            className={`segment-sm ${settings.watch_mode === mode ? 'active' : ''}`}
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
                        <div className="settings-section">
                            {agents.map((agent) => (
                                <div key={agent.name} className={`agent-card ${!agent.available ? 'agent-unavailable' : ''}`}>
                                    <div className="agent-info">
                                        <div className="agent-name-row">
                                            <span className="agent-name">{agent.name}</span>
                                            <span className="agent-source-badge">{agent.source_type}</span>
                                        </div>
                                        <span className={`agent-status ${agent.available ? 'available' : 'unavailable'}`}>
                                            {agent.available
                                                ? t(agent.enabled ? 'settings.enabled' : 'settings.disabled')
                                                : t('settings.notDetected')}
                                        </span>
                                    </div>
                                    <label className="toggle">
                                        <input
                                            type="checkbox"
                                            checked={agent.enabled}
                                            disabled={!agent.available}
                                            onChange={() => handleToggleAgent(agent.name, !agent.enabled)}
                                        />
                                        <span className="toggle-slider" />
                                    </label>
                                </div>
                            ))}
                        </div>
                    )}

                    {tab === 'data' && settings && (
                        <div className="settings-section">
                            <div className="setting-row">
                                <label>{t('settings.keepDays')}</label>
                                <div className="keep-days-input">
                                    <input
                                        type="number"
                                        min={1}
                                        max={365}
                                        value={settings.keep_days}
                                        onChange={(e) => updateSetting('keep_days', e.target.value)}
                                    />
                                    <span>{t('settings.days')}</span>
                                </div>
                            </div>
                            <div className="setting-actions">
                                <button
                                    className="btn-secondary"
                                    onClick={() => setConfirmAction('clearOld')}
                                >
                                    {t('settings.clearOld')}
                                </button>
                                <button
                                    className="btn-danger"
                                    onClick={() => setConfirmAction('clearAll')}
                                >
                                    {t('settings.clearAll')}
                                </button>
                            </div>

                            {confirmAction && (
                                <div className="confirm-dialog">
                                    <p>{t('settings.clearConfirm')}</p>
                                    <div className="confirm-actions">
                                        <button
                                            className="btn-secondary"
                                            onClick={() => setConfirmAction(null)}
                                        >
                                            {t('settings.cancel')}
                                        </button>
                                        <button
                                            className="btn-danger"
                                            onClick={() => handleClearData(confirmAction === 'clearAll')}
                                        >
                                            {t('settings.confirm')}
                                        </button>
                                    </div>
                                </div>
                            )}
                        </div>
                    )}
                </motion.div>
            </AnimatePresence>

            {import.meta.env.DEV && (
                <div className="dev-mode-badge">DEV MODE</div>
            )}
        </div>
    );
}

export default Settings;
