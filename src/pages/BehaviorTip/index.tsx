import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { AnimatePresence, motion } from 'framer-motion';
import type { BehaviorTip as BehaviorTipPayload } from '../../types';
import './index.css';

export function agentLabel(agentName: string): string {
    if (agentName === 'codex') return 'Codex';
    if (agentName === 'opencode') return 'OpenCode';
    return agentName;
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

function BehaviorTip() {
    const [tip, setTip] = useState<BehaviorTipPayload | null>(null);
    const tone = useMemo(() => tip ? kindClass(tip.kind) : 'neutral', [tip]);
    const tipKey = tip?.key;
    const autoHideMs = tip?.auto_hide_ms;

    useEffect(() => {
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
                        <div className="behavior-tip-status" aria-hidden="true" />
                        <div className="behavior-tip-copy">
                            <div className="behavior-tip-meta">{agentLabel(tip.agent_name)}</div>
                            <div className="behavior-tip-title">{tip.title}</div>
                            <div className="behavior-tip-summary">{tip.summary}</div>
                        </div>
                        <button
                            type="button"
                            className="behavior-tip-close"
                            aria-label="Close"
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
