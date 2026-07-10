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
import './index.css';

const TIME_RANGES: { key: TimeRange; labelKey: string }[] = [
    { key: 'today', labelKey: 'popup.today' },
    { key: '7d', labelKey: 'popup.week' },
    { key: '30d', labelKey: 'popup.month' },
];

const POPUP_BASE_HEIGHT = 540;
const POPUP_ACCOUNT_USAGE_MAX_HEIGHT = 680;
const POPUP_HEIGHT_BUFFER = 2;

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

export function getPopupWindowHeight(hasAccountUsage: boolean, contentHeight: number): number {
    if (!hasAccountUsage || !Number.isFinite(contentHeight) || contentHeight <= 0) {
        return POPUP_BASE_HEIGHT;
    }

    return Math.max(
        POPUP_BASE_HEIGHT,
        Math.min(Math.ceil(contentHeight) + POPUP_HEIGHT_BUFFER, POPUP_ACCOUNT_USAGE_MAX_HEIGHT),
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

export function Popup() {
    const { t } = useTranslation();
    const { summary, loading, error, refresh, range, setRange } = useToken();
    const { snapshots, providers, reload: reloadAccountUsage } = useAccountUsageContext();
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
    const topModels = getTopModels(summary?.by_model);
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
    }, [resizeToContent, snapshots.length, providers.length, topModels.length, isSummaryLoading, isCostLoading]);

    if (error) {
        return (
            <div className="popup-error">
                <p>{t('common.error')}</p>
                <button type="button" onClick={refresh}>{t('common.retry')}</button>
            </div>
        );
    }

    return (
        <div ref={containerRef} className={`popup-container${isWindows ? ' windows' : ''}`}>
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
                    <span className="summary-value">{isSummaryLoading ? <span className="skeleton-pulse" /> : formatTokenCount(summary?.total ?? 0)}</span>
                    <span className="summary-label">{t('popup.total')}</span>
                </div>
                <div className="summary-item right">
                    <span className="summary-value cost">{isCostLoading ? <span className="skeleton-pulse wide" /> : formatCost(cost)}</span>
                    <span className="summary-label">{t('popup.cost')}</span>
                </div>
            </div>

            {/* Burger */}
            <Burger summary={summary} range={range} themeId={colorTheme} />

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

            {/* Account Usage */}
            <AccountUsageCard />
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
