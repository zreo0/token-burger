import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useToken } from '../../context/TokenContext';
import Burger from '../../components/Burger';
import ErrorBoundary from '../../components/ErrorBoundary';
import { formatTokenCount, formatCost } from '../../utils/format';
import { calculateTotalCost } from '../../utils/pricing';
import type { ColdStartProgress, PricingTable, TimeRange, TokenBreakdown } from '../../types';
import './index.css';

const TIME_RANGES: { key: TimeRange; labelKey: string }[] = [
    { key: 'today', labelKey: 'popup.today' },
    { key: '7d', labelKey: 'popup.week' },
    { key: '30d', labelKey: 'popup.month' },
];

export function getBreakdownTotal(breakdown: TokenBreakdown): number {
    return breakdown.input + breakdown.cache_create + breakdown.cache_read + breakdown.output;
}

export function getTopModels(byModel: Record<string, TokenBreakdown> | null | undefined) {
    if (!byModel) {
        return [];
    }

    return Object.entries(byModel)
        .sort(([, a], [, b]) => getBreakdownTotal(b) - getBreakdownTotal(a))
        .slice(0, 2);
}

export function Popup() {
    const { t } = useTranslation();
    const { summary, loading, error, refresh, range, setRange } = useToken();
    const [coldStart, setColdStart] = useState<ColdStartProgress | null>(null);
    const [pricing, setPricing] = useState<PricingTable>({});
    const [pricingReady, setPricingReady] = useState(false);

    useEffect(() => {
        invoke<PricingTable>('get_pricing')
            .then(setPricing)
            .catch(() => {})
            .finally(() => setPricingReady(true));
    }, []);

    useEffect(() => {
        const unlisten = listen<ColdStartProgress>('cold-start-progress', (event) => {
            const progress = event.payload;

            if (progress.completed >= progress.total) {
                setColdStart(null);

                return;
            }

            setColdStart(progress);
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, []);

    const cost = summary ? calculateTotalCost(summary.by_model, pricing) : 0;
    const loadingLabel = t('popup.loading');
    const isSummaryLoading = loading || !summary;
    const isCostLoading = loading || !pricingReady;
    const topModels = getTopModels(summary?.by_model);

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

            {/* 顶部摘要 */}
            <div className="top-summary">
                <div className="summary-item">
                    <span className="summary-value">{isSummaryLoading ? loadingLabel : formatTokenCount(summary?.total ?? 0)}</span>
                    <span className="summary-label">{t('popup.total')}</span>
                </div>
                <div className="summary-item right">
                    <span className="summary-value cost">{isCostLoading ? loadingLabel : formatCost(cost)}</span>
                    <span className="summary-label">{t('popup.cost')}</span>
                </div>
            </div>

            {/* Burger */}
            <Burger summary={summary} range={range} />

            {/* Top Models */}
            {topModels.length > 0 && (
                <div className="top-models">
                    <div className="models-header">{t('popup.top_models')}</div>
                    {topModels.map(([model, counts]) => (
                        <div key={model} className="model-row">
                            <span className="model-name">{model}</span>
                            <span className="model-count">{formatTokenCount(getBreakdownTotal(counts))}</span>
                        </div>
                    ))}
                </div>
            )}

            {coldStart && (
                <div className="cold-start-light">
                    {t('popup.coldStart', { agent: coldStart.agent })} ({coldStart.completed}/{coldStart.total})
                </div>
            )}
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
