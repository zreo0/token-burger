import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { PricingTable, TimeRange, TokenBreakdown, TokenTrend } from '../../types';
import { calculateTotalCost } from '../../utils/pricing';
import { formatCost, formatTokenCount } from '../../utils/format';

/**
 * 汇总模型用量，参数为模型细分，返回 Token 总数
 */
export function totalTokens(models: Record<string, TokenBreakdown>): number {
    return Object.values(models).reduce((sum, value) => sum + value.input + value.output + value.cache_read + value.cache_create, 0);
}

/**
 * 计算同期变化，基数为零或历史不足时返回 null
 */
export function getChangePercent(current: number, previous: number, available: boolean): string | null {
    if (!available || previous <= 0) return null;
    const change = (current - previous) / previous * 100;
    return `${change > 0 ? '+' : ''}${Math.round(change)}%`;
}

/**
 * 渲染真实时间桶，支持 Token／费用切换以及鼠标和键盘查看明细
 */
export default function TrendChart({ trend, pricing, range, loading, error, onRetry }: {
    trend: TokenTrend | null;
    pricing: PricingTable;
    range: TimeRange;
    loading: boolean;
    error: boolean;
    onRetry: () => void;
}) {
    const { t, i18n } = useTranslation();
    const [metric, setMetric] = useState<'tokens' | 'cost'>('tokens');
    const [active, setActive] = useState<number | null>(null);
    const buckets = trend?.buckets ?? [];
    const values = buckets.map(bucket => metric === 'tokens' ? totalTokens(bucket.by_model) : calculateTotalCost(bucket.by_model, pricing));
    const max = Math.max(...values, metric === 'tokens' ? 1 : 0.01);
    const format = metric === 'tokens' ? formatTokenCount : formatCost;
    const dateFormat = new Intl.DateTimeFormat(i18n?.language ?? 'en', range === 'today'
        ? { hour: '2-digit', minute: '2-digit', hour12: false }
        : { month: 'numeric', day: 'numeric' });
    const selected = active === null ? undefined : buckets[active];
    /**
     * 生成时间桶说明，日桶展示完整自然日边界，末桶提示今天尚未结束
     */
    const bucketLabel = (index: number): string => {
        const bucket = buckets[index];
        const date = dateFormat.format(bucket.start * 1000);
        const interval = range === 'today'
            ? `${date} – ${dateFormat.format(bucket.end * 1000)}`
            : `${date} 00:00 – 24:00${index === buckets.length - 1 ? ` · ${t('popup.todayIncomplete')}` : ''}`;
        return `${interval} · ${format(values[index])}`;
    };

    return (
        <section className="trend-section" aria-busy={loading}>
            <div className="section-heading">
                <h2>{t('popup.trend')}</h2>
                <div className="mini-segments" aria-label={t('popup.trend')}>
                    <button type="button" aria-pressed={metric === 'tokens'} onClick={() => setMetric('tokens')}>Token</button>
                    <button type="button" aria-pressed={metric === 'cost'} onClick={() => setMetric('cost')}>{t('popup.expense')}</button>
                </div>
            </div>
            <div className="trend-chart">
                <div className="chart-axis"><span>{format(max)}</span><span>{format(max / 2)}</span><span>0</span></div>
                <div className="chart-plot">
                    <div className="chart-grid" aria-hidden="true"><i /><i /><i /></div>
                    {loading || error || !values.some(value => value > 0) ? (
                        <div className="chart-message" role="status">
                            {loading ? t('common.loading') : error ? <button type="button" onClick={onRetry}>{t('popup.trendError')} · {t('common.retry')}</button> : t('popup.noTokens')}
                        </div>
                    ) : (
                        <div className="chart-bars" onMouseLeave={() => setActive(null)}>
                            {buckets.map((bucket, index) => (
                                <button key={bucket.start} type="button" className="chart-bar-hit"
                                    aria-label={bucketLabel(index)}
                                    onMouseEnter={() => setActive(index)} onFocus={() => setActive(index)} onBlur={() => setActive(null)}>
                                    <span className="chart-bar" style={{ height: `${values[index] / max * 100}%` }} />
                                </button>
                            ))}
                        </div>
                    )}
                    {selected && !loading && !error && <div className="chart-tooltip" role="status">{bucketLabel(active!)}</div>}
                </div>
                <div className="chart-times">
                    <span>{buckets.length ? dateFormat.format(buckets[0].start * 1000) : '—'}</span>
                    <span>{buckets.length ? dateFormat.format(buckets[Math.floor(buckets.length / 2)].start * 1000) : ''}</span>
                    <span>{range === 'today' ? t('popup.now') : buckets.length ? `${dateFormat.format(buckets[buckets.length - 1].start * 1000)} · ${t('popup.today')}` : '—'}</span>
                </div>
            </div>
        </section>
    );
}
