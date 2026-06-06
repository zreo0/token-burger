import { useEffect, useLayoutEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { AnimatePresence, motion } from 'framer-motion';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import type { BehaviorTip as BehaviorTipPayload } from '../../types';
import claudeCodeProviderIcon from '../../assets/provider-icons/claude-code.svg';
import githubCopilotProviderIcon from '../../assets/provider-icons/github-copilot.svg';
import openaiProviderIcon from '../../assets/provider-icons/openai.svg';
import opencodeProviderIcon from '../../assets/provider-icons/opencode.svg';
import './index.css';

const SUMMARY_KEYS: Record<string, string> = {
    'codex:A new turn started': 'behaviorTip.summary.codexTurnStarted',
    'codex:Codex is waiting for permission': 'behaviorTip.summary.codexPermissionRequested',
    'codex:Permission request was handled': 'behaviorTip.summary.codexPermissionResolved',
    'codex:Codex finished the current turn': 'behaviorTip.summary.codexRunCompleted',
    'codex:Codex stopped the current turn': 'behaviorTip.summary.codexRunAborted',
    'opencode:OpenCode finished the current turn': 'behaviorTip.summary.opencodeRunCompleted',
};

const CODEX_ABORT_PREFIX = 'Codex stopped the current turn: ';
const AGENT_ICONS: Record<string, string> = {
    codex: openaiProviderIcon,
    'claude-code': claudeCodeProviderIcon,
    'github-copilot': githubCopilotProviderIcon,
    opencode: opencodeProviderIcon,
};

export function agentLabel(agentName: string): string {
    if (agentName === 'codex') return 'Codex';
    if (agentName === 'opencode') return 'OpenCode';
    return agentName;
}

export function agentIcon(agentName: string): string | null {
    return AGENT_ICONS[agentName] ?? null;
}

export function kindClass(kind: BehaviorTipPayload['kind']): string {
    if (kind === 'permission_requested') return 'permission';
    if (kind === 'run_completed') return 'success';
    if (kind === 'run_aborted' || kind === 'tool_error') return 'warning';
    return 'neutral';
}

export function shouldAutoHide(tip: BehaviorTipPayload): boolean {
    return typeof tip.auto_hide_ms === 'number' && tip.auto_hide_ms > 0;
}

export function localizedTipTitle(t: TFunction, tip: BehaviorTipPayload): string {
    return t(`behaviorTip.title.${tip.kind}`, { defaultValue: tip.title });
}

export function localizedTipSummary(t: TFunction, tip: BehaviorTipPayload): string {
    const summaryKey = SUMMARY_KEYS[`${tip.agent_name}:${tip.summary}`];
    if (summaryKey) return t(summaryKey);

    if (tip.agent_name === 'codex' && tip.summary.startsWith(CODEX_ABORT_PREFIX)) {
        return t('behaviorTip.summary.codexRunAbortedWithReason', {
            reason: tip.summary.slice(CODEX_ABORT_PREFIX.length),
        });
    }

    return tip.summary;
}

function BehaviorTip() {
    const { t } = useTranslation();
    const [tip, setTip] = useState<BehaviorTipPayload | null>(null);
    const tone = useMemo(() => tip ? kindClass(tip.kind) : 'neutral', [tip]);
    const icon = useMemo(() => tip ? agentIcon(tip.agent_name) : null, [tip]);
    const tipKey = tip?.key;
    const autoHideMs = tip?.auto_hide_ms;

    useLayoutEffect(() => {
        document.documentElement.classList.add('popup-window-root');
        document.body.classList.add('popup-window');

        return () => {
            document.documentElement.classList.remove('popup-window-root');
            document.body.classList.remove('popup-window');
        };
    }, []);

    useEffect(() => {
        invoke<BehaviorTipPayload | null>('get_current_behavior_tip')
            .then((current) => setTip(current))
            .catch(() => {});

        const unlisten = listen<BehaviorTipPayload | null>('behavior-tip-updated', (event) => {
            setTip(event.payload ?? null);
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, []);

    useEffect(() => {
        if (!tip || !shouldAutoHide(tip)) return;
        if (typeof autoHideMs !== 'number') return;

        const timer = window.setTimeout(() => {
            invoke('close_behavior_tip').catch(() => {});
        }, autoHideMs);

        return () => window.clearTimeout(timer);
    }, [tip, tipKey, autoHideMs]);

    const close = () => {
        invoke('close_behavior_tip').catch(() => {});
    };

    return (
        <div className="behavior-tip-shell">
            <AnimatePresence mode="wait">
                {tip && (
                    <motion.div
                        key={tip.key}
                        className={`behavior-tip-card ${tone}`}
                        initial={{ opacity: 0, y: -6, scale: 0.985 }}
                        animate={{ opacity: 1, y: 0, scale: 1 }}
                        exit={{ opacity: 0, y: -4, scale: 0.985 }}
                        transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
                    >
                        <div className="behavior-tip-icon-wrap" aria-hidden="true">
                            {icon ? (
                                <img
                                    className="behavior-tip-icon"
                                    src={icon}
                                    alt=""
                                />
                            ) : (
                                <span className="behavior-tip-icon-fallback">
                                    {agentLabel(tip.agent_name).slice(0, 2).toUpperCase()}
                                </span>
                            )}
                            <span className="behavior-tip-status" />
                        </div>
                        <div className="behavior-tip-copy">
                            <div className="behavior-tip-heading">
                                <span className="behavior-tip-agent">{agentLabel(tip.agent_name)}</span>
                                <span className="behavior-tip-dot" aria-hidden="true">·</span>
                                <span className="behavior-tip-title">{localizedTipTitle(t, tip)}</span>
                            </div>
                            <div className="behavior-tip-summary">{localizedTipSummary(t, tip)}</div>
                        </div>
                        <button
                            type="button"
                            className="behavior-tip-close"
                            aria-label={t('behaviorTip.close')}
                            onClick={close}
                        >
                            ×
                        </button>
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
}

export default BehaviorTip;
