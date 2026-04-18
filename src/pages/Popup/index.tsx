import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useToken } from '../../context/TokenContext';
import Burger from '../../components/Burger';
import ErrorBoundary from '../../components/ErrorBoundary';
import { formatTokenCount, formatCost } from '../../utils/format';
import { calculateTotalCost } from '../../utils/pricing';
import type { ColdStartProgress, PricingTable, TimeRange } from '../../types';
import './index.css';

const TIME_RANGES: { key: TimeRange; labelKey: string }[] = [
    { key: 'today', labelKey: 'popup.today' },
    { key: '7d', labelKey: 'popup.week' },
    { key: '30d', labelKey: 'popup.month' },
];

function Popup() {
    const { t } = useTranslation();
    const { summary, loading, error, refresh, range, setRange } = useToken();
    const [coldStart, setColdStart] = useState<ColdStartProgress | null>(null);
    const [pricing, setPricing] = useState<PricingTable>({});

    useEffect(() => {
        invoke<PricingTable>('get_pricing').then(setPricing).catch(() => {});
    }, []);

    useEffect(() => {
        const unlisten = listen<ColdStartProgress>('cold-start-progress', (event) => {
            const progress = event.payload;
            setColdStart(progress);
            if (progress.completed >= progress.total) {
                setTimeout(() => setColdStart(null), 1000);
            }
        });
        return () => {
            unlisten.then((fn) => fn());
        };
    }, []);

    const cost = summary ? calculateTotalCost(summary.by_model, pricing) : 0;

    if (error) {
        return (
            <div className="popup-error">
                <p>{t('common.error')}</p>
                <button type="button" onClick={refresh}>{t('common.retry')}</button>
            </div>
        );
    }

    return (
        <div className="popup-container">
            {/* 时间范围选择器 */}
            <div className="segmented-control">
                {TIME_RANGES.map(({ key, labelKey }) => (
                    <button
                        type="button"
                        key={key}
                        className={`segment ${range === key ? 'active' : ''}`}
                        onClick={() => setRange(key)}
                    >
                        {t(labelKey)}
                    </button>
                ))}
            </div>

            {/* 冷启动状态 */}
            {coldStart && (
                <div className="cold-start-banner">
                    {t('popup.coldStart', { agent: coldStart.agent })}
                    <span className="cold-start-progress">
                        {coldStart.completed}/{coldStart.total}
                    </span>
                </div>
            )}

            {/* Burger */}
            <Burger summary={summary} />

            {/* 底部汇总 */}
            <div className="popup-footer">
                <div className="footer-item">
                    <span className="footer-label">{t('popup.total')}</span>
                    <span className="footer-value">
                        {loading ? '...' : formatTokenCount(summary?.total ?? 0)}
                    </span>
                </div>
                <div className="footer-item">
                    <span className="footer-label">{t('popup.cost')}</span>
                    <span className="footer-value cost">
                        {loading ? '...' : formatCost(cost)}
                    </span>
                </div>
            </div>
        </div>
    );
}

function PopupPage() {
    return (
        <ErrorBoundary>
            <Popup />
        </ErrorBoundary>
    );
}

export default PopupPage;
