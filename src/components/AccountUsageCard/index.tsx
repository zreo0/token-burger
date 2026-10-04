import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import { useAccountUsageContext } from '../../context/AccountUsageContext';
import { formatTokenCount } from '../../utils/format';
import type { AccountUsageMetric, AccountUsageProviderInfo, AccountUsageSnapshot } from '../../types';
import './index.css';

const RESET_CREDIT_METRIC_KEY = 'codex.reset_credits.available';

const PLAN_BADGE_LABELS: Record<string, Record<string, string>> = {
    codex: {
        prolite: '5x',
    },
};

type Translate = TFunction;

export function getAccountUsagePlanBadge(providerId: string, plan?: string | null): string | null {
    const normalizedPlan = plan?.trim();
    if (!normalizedPlan) return null;

    return PLAN_BADGE_LABELS[providerId]?.[normalizedPlan.toLowerCase()] ?? normalizedPlan;
}

export function formatAccountUsageMetricValue(metric: AccountUsageMetric): string {
    if (metric.percentage != null) {
        return `${metric.percentage.toFixed(1)}%`;
    }
    if (metric.unit === 'reset_credit') {
        const count = metric.remaining ?? metric.used ?? 0;
        return String(Math.round(count));
    }
    if (metric.used != null) {
        if (metric.unit === 'token' || metric.unit === 'tokens') {
            return formatTokenCount(metric.used);
        }
        if (metric.unit === 'usd' || metric.unit === 'USD') {
            return `$${metric.used.toFixed(2)}`;
        }
        return `${metric.used} ${metric.unit}`;
    }
    if (metric.remaining != null) {
        return `${metric.remaining} ${metric.unit}`;
    }
    return '0';
}

export function formatAccountUsageResetTime(resetAt?: string | null, now = new Date()): string | null {
    if (!resetAt) return null;
    const resetTime = new Date(resetAt).getTime();
    if (!Number.isFinite(resetTime)) return null;

    const totalMinutes = Math.max(0, Math.floor((resetTime - now.getTime()) / 60000));
    const days = Math.floor(totalMinutes / 1440);
    const hours = Math.floor((totalMinutes % 1440) / 60);
    const minutes = totalMinutes % 60;

    if (days > 0) return `${days}d${hours}h${minutes}m`;
    if (hours > 0) return `${hours}h${minutes}m`;
    return `${minutes}m`;
}

function getMetricPercent(metric: AccountUsageMetric): number | null {
    const percent = metric.percentage ?? (
        metric.used != null && metric.limit != null && metric.limit > 0
            ? (metric.used / metric.limit) * 100
            : undefined
    );
    if (percent === undefined || !Number.isFinite(percent)) return null;
    return Math.max(0, Math.min(100, percent));
}

function isQuotaMetric(metric: AccountUsageMetric): boolean {
    return getMetricPercent(metric) !== null && (metric.unit === 'percent' || metric.limit !== undefined);
}

function isResetCreditMetric(metric: AccountUsageMetric): boolean {
    return metric.metric_key === RESET_CREDIT_METRIC_KEY || metric.unit === 'reset_credit';
}

function getAccountUsageMetricLabel(metric: AccountUsageMetric, t: Translate): string {
    if (isResetCreditMetric(metric)) {
        return t('usage.resetCredits', 'Reset credits');
    }
    const window = metric.label.match(/^(\d+)(h|d) window$/);
    if (window) return t(window[2] === 'h' ? 'usage.hourWindow' : 'usage.dayWindow', metric.label, { count: Number(window[1]) });
    return metric.label;
}

/**
 * 将剩余时间转换为本地化短文本，参数为时间和翻译器，返回倒计时
 */
function localizedResetTime(resetAt: string | null | undefined, now: Date, t: Translate): string | null {
    const raw = formatAccountUsageResetTime(resetAt, now);
    const value = raw?.includes('d') ? raw.replace(/\d+m$/, '') : raw;
    if (!value) return null;
    return value.replace(/(\d+)d/g, (_, count) => t('usage.daysShort', '{{count}}d ', { count }))
        .replace(/(\d+)h/g, (_, count) => t('usage.hoursShort', '{{count}}h ', { count }))
        .replace(/(\d+)m/g, (_, count) => t('usage.minutesShort', '{{count}}m', { count })).trim();
}

function progressTone(percent: number): string {
    if (percent >= 95) return 'danger';
    if (percent >= 80) return 'warning';
    return 'ok';
}

/**
 * 展示单个额度窗口，参数为指标、当前时间与翻译器，返回进度和重置说明
 */
function QuotaMetric({ metric, now, t }: { metric: AccountUsageMetric; now: Date; t: Translate }) {
    const percent = getMetricPercent(metric) ?? 0;
    const resetTime = localizedResetTime(metric.reset_at, now, t);
    const label = getAccountUsageMetricLabel(metric, t);
    return (
        <div className="usage-quota-metric">
            <span className="usage-metric-label">{label}</span>
            <div className="usage-progress-track" role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}>
                <div className={`usage-progress-fill ${progressTone(percent)}`} style={{ transform: `scaleX(${percent / 100})` }} />
            </div>
            <div className="usage-metric-stats">
                <span className="usage-metric-value">{t('usage.usedPercent', 'Used {{percent}}%', { percent: percent.toFixed(1) })}</span>
                {resetTime && <span className="usage-reset-time">{t('usage.resetsIn', 'Resets in {{time}}', { time: resetTime })}</span>}
            </div>
        </div>
    );
}

function SummaryMetric({ metric, now, t }: { metric: AccountUsageMetric; now: Date; t: Translate }) {
    const resetTime = localizedResetTime(metric.reset_at, now, t);

    return (
        <span className="usage-summary-pill">
            <span className="usage-summary-label">{getAccountUsageMetricLabel(metric, t)}</span>
            <span className="usage-summary-value">{formatAccountUsageMetricValue(metric)}</span>
            {resetTime && <span className="usage-summary-reset">{t('usage.resetsIn', 'Resets in {{time}}', { time: resetTime })}</span>}
        </span>
    );
}

function RefreshIconButton({
    refreshing,
    onClick,
    t,
}: {
    refreshing: boolean;
    onClick: () => void;
    t: Translate;
}) {
    const label = refreshing ? t('common.loading', 'Loading') : t('common.refresh', 'Refresh');

    return (
        <button
            type="button"
            className="usage-refresh-icon"
            onClick={onClick}
            disabled={refreshing}
            title={label}
            aria-label={label}
        >
            <svg className={refreshing ? 'refresh-spinning' : ''} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true"><path d="M20 7v5h-5M4 17v-5h5" /><path d="M19 11a7 7 0 0 0-12-5L4 9m1 4a7 7 0 0 0 12 5l3-3" /></svg>
        </button>
    );
}

function EmptyProviderCard({
    provider,
    errorMessage,
    refreshing,
    refreshProvider,
    t,
}: {
    provider: AccountUsageProviderInfo;
    errorMessage?: string;
    refreshing: boolean;
    refreshProvider: (providerId: string) => void;
    t: Translate;
}) {
    return (
        <article className="usage-provider-card empty">
            <div className="usage-provider-heading">
                <span className="usage-provider-name">{provider.display_name}</span>
                <RefreshIconButton refreshing={refreshing} onClick={() => refreshProvider(provider.id)} t={t} />
            </div>
            <p className={errorMessage ? 'usage-error-text' : 'usage-muted-text'}>
                {errorMessage || (provider.available
                    ? t('usage.noData', 'No account usage data yet')
                    : t('usage.notDetected', 'Auth file or credential not detected'))}
            </p>
        </article>
    );
}

function ProviderUsageCard({
    snapshot,
    provider,
    refreshing,
    refreshProvider,
    refreshError,
    t,
    now,
}: {
    snapshot: AccountUsageSnapshot;
    provider?: AccountUsageProviderInfo;
    refreshing: boolean;
    refreshProvider: (providerId: string) => void;
    t: Translate;
    now: Date;
    refreshError?: string;
}) {
    const metrics = snapshot.metrics ?? [];
    const quotaMetrics = metrics.filter(isQuotaMetric);
    const summaryMetrics = metrics.filter(metric => !isQuotaMetric(metric) && !isResetCreditMetric(metric));
    const hasError = snapshot.status === 'error' || snapshot.status === 'auth_required' || snapshot.status === 'forbidden';
    const resetCreditMetric = metrics.find(isResetCreditMetric);
    const creditExpiry = resetCreditMetric ? localizedResetTime(resetCreditMetric.reset_at, now, t) : null;
    const [expanded, setExpanded] = useState(true);
    const planBadge = getAccountUsagePlanBadge(snapshot.provider_id, snapshot.plan);

    return (
        <article className="usage-provider-card">
            <div className="usage-provider-heading">
                <div className="usage-provider-title-row">
                    <span className="usage-provider-name">{provider?.display_name || snapshot.provider_id}</span>
                    {planBadge && <span className="usage-plan-badge">{planBadge}</span>}
                    {snapshot.stale && <span className="usage-stale-text">{t('usage.stale', 'Stale')}</span>}
                </div>
                <div className="usage-provider-actions">
                    <RefreshIconButton refreshing={refreshing} onClick={() => refreshProvider(snapshot.provider_id)} t={t} />
                    <button className="usage-expand" type="button" aria-label={t(expanded ? 'usage.collapse' : 'usage.expand', expanded ? 'Collapse account details' : 'Expand account details')} aria-expanded={expanded} onClick={() => setExpanded(value => !value)}>
                        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4" aria-hidden="true"><path d={expanded ? 'm3 6 5 5 5-5' : 'm6 3 5 5-5 5'} /></svg>
                    </button>
                </div>
            </div>

            {hasError ? (
                <p className="usage-error-text">{snapshot.error?.message || snapshot.status}</p>
            ) : expanded ? (
                <>
                    {quotaMetrics.length > 0 && (
                        <div className="usage-quota-list">
                            {quotaMetrics.map(metric => (
                                <QuotaMetric key={`${metric.metric_key}-${metric.scope}`} metric={metric} now={now} t={t} />
                            ))}
                        </div>
                    )}
                    {resetCreditMetric && (
                        <div className="usage-credit-detail">
                            <div><span>{t('usage.availableResets', 'Available resets')}</span><strong>{formatAccountUsageMetricValue(resetCreditMetric)}</strong></div>
                            {creditExpiry && <p>{t('usage.creditExpires', 'Next credit expires in {{time}}', { time: creditExpiry })}</p>}
                        </div>
                    )}
                    {refreshError && <p className="usage-error-text" role="alert">{refreshError}</p>}
                    {snapshot.error && <p className="usage-error-text">{snapshot.error.message}</p>}
                    {!metrics.length && <p className="usage-muted-text">{t('usage.noData', 'No account usage data yet')}</p>}
                    {summaryMetrics.length > 0 && (
                        <div className="usage-summary-list">
                            {summaryMetrics.map(metric => (
                                <SummaryMetric key={`${metric.metric_key}-${metric.scope}`} metric={metric} now={now} t={t} />
                            ))}
                        </div>
                    )}
                    <details className="usage-extra-details">
                        <summary>{t('usage.details', 'Details')}</summary>
                        {snapshot.account_label && <p>{snapshot.account_label}</p>}
                        <p>{t('popup.updatedAt', 'Updated {{time}}', { time: new Date(snapshot.observed_at).toLocaleString() })}</p>
                        {quotaMetrics.filter(metric => metric.limit != null && metric.unit !== 'percent').map(metric => <p key={metric.metric_key}>{metric.label}: {metric.used ?? '—'} / {metric.limit} {metric.unit}</p>)}
                    </details>
                </>
            ) : <p className="usage-muted-text">{quotaMetrics.map(metric => `${getAccountUsageMetricLabel(metric, t)} · ${formatAccountUsageMetricValue(metric)}`).join(' / ')}</p>}
        </article>
    );
}

export default function AccountUsageCard() {
    const { t } = useTranslation();
    const { snapshots, providers, refreshing, refreshingProviders, providerErrors, refreshAll, refreshProvider } = useAccountUsageContext();
    const [now, setNow] = useState(() => new Date());
    const enabledProviderIds = new Set(providers.filter(provider => provider.enabled).map(provider => provider.id));
    const visibleSnapshots = snapshots.filter(snapshot => enabledProviderIds.has(snapshot.provider_id));
    const enabledProvidersWithoutSnapshots = providers.filter(provider => (
        provider.enabled && !visibleSnapshots.some(snapshot => snapshot.provider_id === provider.id)
    ));

    useEffect(() => {
        if (visibleSnapshots.length === 0) return;

        const timer = window.setInterval(() => setNow(new Date()), 60000);

        return () => window.clearInterval(timer);
    }, [visibleSnapshots.length]);

    if (visibleSnapshots.length === 0 && enabledProvidersWithoutSnapshots.length === 0) return null;

    return (
        <section className="account-usage-card">
            <div className="usage-card-header">
                <span>{t('usage.title', 'Account Usage')}</span>
                <RefreshIconButton refreshing={refreshing} onClick={refreshAll} t={t} />
            </div>

            <div className="usage-card-grid">
                {enabledProvidersWithoutSnapshots.map(provider => (
                    <EmptyProviderCard
                        key={`${provider.id}-empty`}
                        provider={provider}
                        errorMessage={providerErrors[provider.id]}
                        refreshing={!!refreshingProviders[provider.id]}
                        refreshProvider={refreshProvider}
                        t={t}
                    />
                ))}

                {visibleSnapshots.map(snapshot => (
                    <ProviderUsageCard
                        key={`${snapshot.provider_id}-${snapshot.account_key}`}
                        snapshot={snapshot}
                        provider={providers.find(provider => provider.id === snapshot.provider_id)}
                        refreshing={!!refreshingProviders[snapshot.provider_id]}
                        refreshError={providerErrors[snapshot.provider_id]}
                        refreshProvider={refreshProvider}
                        t={t}
                        now={now}
                    />
                ))}
            </div>
        </section>
    );
}
