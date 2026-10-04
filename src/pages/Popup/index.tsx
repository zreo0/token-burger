import { useState, useEffect, useLayoutEffect, useRef, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useToken } from '../../context/TokenContext';
import { useAccountUsageContext } from '../../context/AccountUsageContext';
import Burger from '../../components/Burger';
import AccountUsageCard from '../../components/AccountUsageCard';
import ErrorBoundary from '../../components/ErrorBoundary';
import { formatTokenCount, formatCost } from '../../utils/format';
import { calculateTotalCost } from '../../utils/pricing';
import { getPlatformInfo } from '../../utils/platform';
import { DEFAULT_THEME_ID } from '../../components/Burger/themes';
import type { AppSettings, PricingTable, TimeRange, TokenBreakdown } from '../../types';
import TrendChart, { getChangePercent, totalTokens } from './trend-chart';
import type { TokenTrend } from '../../types';
import './index.css';

const TIME_RANGES: { key: TimeRange; labelKey: string }[] = [
    { key: 'today', labelKey: 'popup.today' },
    { key: '7d', labelKey: 'popup.week' },
    { key: '30d', labelKey: 'popup.month' },
];

const POPUP_BASE_HEIGHT = 600;
const POPUP_ACCOUNT_USAGE_MAX_HEIGHT = 860;
const POPUP_HEIGHT_BUFFER = 2;

/**
 * 根据四类用量返回来源总数
 */
export function getBreakdownTotal(breakdown: TokenBreakdown): number {
    return breakdown.input + breakdown.cache_create + breakdown.cache_read + breakdown.output;
}

/**
 * 根据内容高度与账号区是否可见返回期望窗口高度，屏幕边界由后端限制
 */
export function getPopupWindowHeight(hasAccountUsage: boolean, contentHeight: number): number {
    if (!Number.isFinite(contentHeight) || contentHeight <= 0) {
        return POPUP_BASE_HEIGHT;
    }

    return Math.max(
        POPUP_BASE_HEIGHT,
        Math.min(Math.ceil(contentHeight) + POPUP_HEIGHT_BUFFER, hasAccountUsage ? POPUP_ACCOUNT_USAGE_MAX_HEIGHT : 740),
    );
}

/**
 * 监听模型价格更新事件
 * 参数为价格更新回调，返回事件取消监听函数 Promise
 */
export function listenForPricingUpdates(onUpdate: () => void): Promise<UnlistenFn> {
    return listen('pricing-updated', () => {
        onUpdate();
    });
}

/**
 * 展示用量、趋势与账号额度，无参数，返回菜单栏弹窗
 */
export function Popup() {
    const { t } = useTranslation();
    const { summary, loading, error, refresh, range, setRange } = useToken();
    const { snapshots, providers, reload: reloadAccountUsage } = useAccountUsageContext();
    const [trendResult, setTrendResult] = useState<{ range: TimeRange; data: TokenTrend } | null>(null);
    const [trendLoading, setTrendLoading] = useState(true);
    const [trendError, setTrendError] = useState(false);
    const [trendRetry, setTrendRetry] = useState(0);
    const [source, setSource] = useState<'model' | 'agent'>('model');
    const [showAll, setShowAll] = useState(false);
    const [settingsError, setSettingsError] = useState(false);
    const [updatedAt, setUpdatedAt] = useState<Date | null>(null);
    const [pricing, setPricing] = useState<PricingTable>({});
    const [pricingReady, setPricingReady] = useState(false);
    const [colorTheme, setColorTheme] = useState(DEFAULT_THEME_ID);
    const [isWindows, setIsWindows] = useState(false);
    const containerRef = useRef<HTMLDivElement | null>(null);
    const lastPopupHeight = useRef<number | null>(null);

    /**
     * 从后端读取当前模型价格表
     * 无参数和返回值，读取结果写入组件状态
     */
    const loadPricing = useCallback(async (): Promise<void> => {
        try {
            const table = await invoke<PricingTable>('get_pricing');
            setPricing(table);
        } catch {
            // 保留当前价格表
        } finally {
            setPricingReady(true);
        }
    }, []);

    useLayoutEffect(() => {
        document.documentElement.classList.add('popup-window-root');
        document.body.classList.add('popup-window');

        return () => {
            document.documentElement.classList.remove('popup-window-root');
            document.body.classList.remove('popup-window');
        };
    }, []);

    useEffect(() => {
        void loadPricing();

        invoke<AppSettings>('get_settings')
            .then((s) => { if (s.color_theme) setColorTheme(s.color_theme); })
            .catch(() => {});

        getPlatformInfo()
            .then((info) => setIsWindows(info.platform === 'windows'))
            .catch(() => {});
    }, [loadPricing]);

    useEffect(() => {
        const unlistenTheme = listen<string>('settings-color-theme-changed', (event) => {
            setColorTheme(event.payload);
        });
        const unlistenPricing = listenForPricingUpdates(() => {
            void loadPricing();
        });

        return () => {
            unlistenTheme.then((fn) => fn());
            unlistenPricing.then((fn) => fn());
        };
    }, [loadPricing]);

    const cost = summary ? calculateTotalCost(summary.by_model, pricing) : 0;
    const isSummaryLoading = loading || !summary;
    const isCostLoading = loading || !pricingReady;
    const allSources = Object.entries((source === 'model' ? summary?.by_model : summary?.by_agent) ?? {})
        .sort(([, a], [, b]) => getBreakdownTotal(b) - getBreakdownTotal(a));
    const visibleSources = showAll ? allSources : allSources.slice(0, 2);
    const trend = trendResult?.range === range ? trendResult.data : null;
    const previousTokens = trend ? totalTokens(trend.previous_by_model) : 0;
    const tokenChange = getChangePercent(summary?.total ?? 0, previousTokens, !!trend?.comparison_available);
    const costChange = getChangePercent(cost, trend ? calculateTotalCost(trend.previous_by_model, pricing) : 0, !!trend?.comparison_available);

    useEffect(() => {
        let disposed = false;
        setTrendLoading(true);
        setTrendError(false);
        invoke<TokenTrend>('get_token_trend', { range }).then(data => {
            if (!disposed) {
                setTrendResult({ range, data });
                setUpdatedAt(new Date());
            }
        }).catch(() => {
            if (!disposed) setTrendError(true);
        }).finally(() => {
            if (!disposed) setTrendLoading(false);
        });
        return () => { disposed = true; };
    }, [range, summary, trendRetry]);

    /**
     * 打开独立设置窗口，失败时在弹窗内保留可重试反馈
     */
    const openSettings = async (): Promise<void> => {
        try {
            await invoke('open_settings');
            setSettingsError(false);
        } catch {
            setSettingsError(true);
        }
    };
    const enabledProviderIds = new Set(providers.filter(provider => provider.enabled).map(provider => provider.id));
    const hasAccountUsage = enabledProviderIds.size > 0 || snapshots.some(snapshot => enabledProviderIds.has(snapshot.provider_id));
    const resizeToContent = useCallback(() => {
        const height = getPopupWindowHeight(
            hasAccountUsage,
            containerRef.current?.scrollHeight ?? POPUP_BASE_HEIGHT,
        );

        if (lastPopupHeight.current === height) return;
        lastPopupHeight.current = height;
        invoke('resize_popup_window', { height }).catch(() => {});
    }, [hasAccountUsage]);

    useEffect(() => {
        const unlistenPopupShown = listen('popup-shown', () => {
            lastPopupHeight.current = null;
            window.requestAnimationFrame(() => {
                resizeToContent();
                window.setTimeout(resizeToContent, 0);
            });
            void refresh();
            void reloadAccountUsage();
            void invoke('ensure_pricing_fresh').catch(() => {});
        });

        return () => {
            unlistenPopupShown.then((fn) => fn());
        };
    }, [refresh, reloadAccountUsage, resizeToContent]);

    useLayoutEffect(() => {
        const container = containerRef.current;
        if (!container) return;

        let frame: number | null = null;
        const queueResize = () => {
            if (frame !== null) {
                window.cancelAnimationFrame(frame);
            }
            frame = window.requestAnimationFrame(() => {
                frame = null;
                resizeToContent();
            });
        };

        queueResize();
        const observer = new ResizeObserver(queueResize);
        observer.observe(container);

        return () => {
            if (frame !== null) {
                window.cancelAnimationFrame(frame);
            }
            observer.disconnect();
        };
    }, [resizeToContent, snapshots.length, providers.length, visibleSources.length, isSummaryLoading, isCostLoading]);

    return (
        <div className={`popup-container${isWindows ? ' windows' : ''}`}>
            <div ref={containerRef} className="popup-content">
                <header className="popup-header">
                    <div className="popup-brand"><span className="burger-logo" aria-hidden="true"><i /><i /><i /><i /></span><h1>TokenBurger</h1></div>
                    <button className="icon-button" type="button" onClick={openSettings} aria-label={t('popup.settings')} title={t('popup.settings')}>
                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true"><path d="m9 3-1 3-3 1 1 3-2 2 2 2-1 3 3 1 1 3h4l1-3 3-1-1-3 2-2-2-2 1-3-3-1-1-3Z" /><circle cx="11" cy="12" r="3" /></svg>
                    </button>
                </header>
                {settingsError && <p className="inline-error" role="alert">{t('popup.settingsError')}</p>}
                <div className="segmented-control" aria-label={t('popup.timeRange')}>
                    {TIME_RANGES.map(({ key, labelKey }) => (
                        <button type="button" key={key} className={`segment ${range === key ? 'active' : ''}`}
                            aria-pressed={range === key} onClick={() => { setRange(key); setShowAll(false); }}>
                            {t(labelKey)}
                        </button>
                    ))}
                </div>
                {error && <div className="inline-error" role="alert">{t('common.error')} <button type="button" onClick={refresh}>{t('common.retry')}</button></div>}
                <section className="top-summary" aria-busy={isSummaryLoading}>
                    <div className="summary-item">
                        <span className="summary-label">{t('popup.total')}</span>
                        <span className="summary-value" title={summary?.total.toLocaleString()}>{error ? '—' : isSummaryLoading ? <span className="skeleton-pulse" /> : formatTokenCount(summary?.total ?? 0, true)}</span>
                        <span className="summary-change">{!error && !loading && !trendLoading && !trendError && tokenChange ? t(range === 'today' ? 'popup.dailyChange' : 'popup.periodChange', { change: tokenChange }) : t('popup.localRecords')}</span>
                    </div>
                    <div className="summary-item">
                        <span className="summary-label">{t('popup.cost')}</span>
                        <span className="summary-value cost" title={t('popup.costHint')}>{error ? '—' : isCostLoading ? <span className="skeleton-pulse" /> : formatCost(cost)}</span>
                        <span className="summary-change">{!error && !loading && !trendLoading && !trendError && costChange ? costChange : t('popup.estimated')}</span>
                    </div>
                </section>
                <TrendChart trend={trend} pricing={pricing} range={range} loading={(trendLoading && !trend) || !pricingReady} error={trendError} onRetry={() => setTrendRetry(value => value + 1)} />
                <section className="composition-section" aria-busy={isSummaryLoading}>
                    <div className="section-heading"><h2>{t('popup.composition')}</h2></div>
                    {isSummaryLoading || error ? <div className="burger-placeholder">{error ? t('common.error') : t('common.loading')}</div> : <Burger summary={summary} range={range} themeId={colorTheme} />}
                </section>
                <section className="top-models">
                    <div className="section-heading">
                        <h2>{t('popup.sources')}</h2>
                        <div className="mini-segments" aria-label={t('popup.sources')}>
                            <button type="button" aria-pressed={source === 'model'} onClick={() => { setSource('model'); setShowAll(false); }}>{t('popup.models')}</button>
                            <button type="button" aria-pressed={source === 'agent'} onClick={() => { setSource('agent'); setShowAll(false); }}>Agent</button>
                        </div>
                    </div>
                    {loading || error ? <p className="muted-text">{error ? t('common.error') : t('common.loading')}</p> : visibleSources.length ? visibleSources.map(([name, counts]) => (
                        <div key={name} className="model-row">
                            <span className="model-name" title={name}>{name}</span>
                            <span className="model-count" title={getBreakdownTotal(counts).toLocaleString()}>{formatTokenCount(getBreakdownTotal(counts), true)}</span>
                            <span className="model-percent">{summary?.total ? Math.round(getBreakdownTotal(counts) / summary.total * 100) : 0}%</span>
                        </div>
                    )) : <p className="muted-text">{t('popup.noTokens')}</p>}
                    {!loading && !error && allSources.length > 2 && <button className="text-button" type="button" aria-expanded={showAll} onClick={() => setShowAll(value => !value)}>{t(showAll ? 'popup.showLess' : 'popup.showAll')} <span aria-hidden="true">{showAll ? '⌃' : '›'}</span></button>}
                </section>
                <AccountUsageCard />
                <footer className="popup-footer"><span>{updatedAt ? t('popup.updatedAt', { time: updatedAt.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false }) }) : t('popup.localRecords')}</span><span>{t('popup.localOnly')}</span></footer>
            </div>
        </div>
    );
}

/**
 * 为弹窗提供渲染错误边界，无参数，返回主界面
 */
function PopupPage() {
    return <ErrorBoundary><Popup /></ErrorBoundary>;
}

export default PopupPage;
