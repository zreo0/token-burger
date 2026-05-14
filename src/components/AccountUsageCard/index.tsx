import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import { useAccountUsageContext } from '../../context/AccountUsageContext';
import { formatTokenCount } from '../../utils/format';
import type { AccountUsageMetric, AccountUsageProviderInfo, AccountUsageSnapshot } from '../../types';
import './index.css';

const MAX_SUMMARY_METRICS = 4;

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

function progressTone(percent: number): string {
    if (percent >= 95) return 'danger';
    if (percent >= 80) return 'warning';
    return 'ok';
}

function QuotaMetric({ metric, now }: { metric: AccountUsageMetric; now: Date }) {
    const percent = getMetricPercent(metric) ?? 0;
    const resetTime = formatAccountUsageResetTime(metric.reset_at, now);

    return (
        <div className="usage-quota-metric">
            <div className="usage-metric-line">
                <span>{metric.label}</span>
                <span className="usage-metric-stats">
                    {resetTime && <span className="usage-reset-time">{resetTime}</span>}
                    <span className="usage-metric-value">{formatAccountUsageMetricValue(metric)}</span>
                </span>
            </div>
            <div className="usage-progress-track" aria-hidden="true">
                <div
                    className={`usage-progress-fill ${progressTone(percent)}`}
                    style={{ width: `${percent}%` }}
                />
            </div>
        </div>
    );
}

function SummaryMetric({ metric }: { metric: AccountUsageMetric }) {
    return (
        <span className="usage-summary-pill">
            <span className="usage-summary-label">{metric.label}</span>
            <span className="usage-summary-value">{formatAccountUsageMetricValue(metric)}</span>
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
            {refreshing ? '…' : '↻'}
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
    t,
    now,
}: {
    snapshot: AccountUsageSnapshot;
    provider?: AccountUsageProviderInfo;
    refreshing: boolean;
    refreshProvider: (providerId: string) => void;
    t: Translate;
    now: Date;
}) {
    const metrics = snapshot.metrics ?? [];
    const quotaMetrics = metrics.filter(isQuotaMetric);
    const summaryMetrics = metrics.filter(metric => !isQuotaMetric(metric)).slice(0, MAX_SUMMARY_METRICS);
    const hasError = snapshot.status === 'error' || snapshot.status === 'auth_required' || snapshot.status === 'forbidden';
    const planBadge = getAccountUsagePlanBadge(snapshot.provider_id, snapshot.plan);

    return (
        <article className="usage-provider-card">
            <div className="usage-provider-heading">
                <div className="usage-provider-title-row">
                    <span className="usage-provider-name">{provider?.display_name || snapshot.provider_id}</span>
                    {planBadge && <span className="usage-plan-badge">{planBadge}</span>}
                    {snapshot.stale && <span className="usage-stale-text">{t('usage.stale', 'Stale')}</span>}
                </div>
                <RefreshIconButton refreshing={refreshing} onClick={() => refreshProvider(snapshot.provider_id)} t={t} />
            </div>

            {hasError ? (
                <p className="usage-error-text">{snapshot.error?.message || snapshot.status}</p>
            ) : (
                <>
                    {quotaMetrics.length > 0 && (
                        <div className="usage-quota-list">
                            {quotaMetrics.map(metric => (
                                <QuotaMetric key={`${metric.metric_key}-${metric.scope}`} metric={metric} now={now} />
                            ))}
                        </div>
                    )}
                    {summaryMetrics.length > 0 && (
                        <div className="usage-summary-list">
                            {summaryMetrics.map(metric => (
                                <SummaryMetric key={`${metric.metric_key}-${metric.scope}`} metric={metric} />
                            ))}
                        </div>
                    )}
                </>
            )}
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
                <button type="button" className="usage-refresh-all" onClick={refreshAll} disabled={refreshing}>
                    {refreshing ? t('common.loading', 'Loading') : t('common.refresh', 'Refresh')}
                </button>
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
                        refreshProvider={refreshProvider}
                        t={t}
                        now={now}
                    />
                ))}
            </div>
        </section>
    );
}
