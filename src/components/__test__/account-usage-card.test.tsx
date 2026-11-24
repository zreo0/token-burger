import { renderToStaticMarkup } from 'react-dom/server';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import AccountUsageCard, { formatAccountUsageMetricValue, formatAccountUsageResetTime, getAccountUsagePlanBadge, getResetCreditEntries } from '../AccountUsageCard';
import type { AccountUsageProviderInfo, AccountUsageSnapshot } from '../../types';

vi.mock('react-i18next', () => ({
    useTranslation: () => ({
        t: (key: string, fallback?: string, options?: Record<string, string | number>) => {
            let value = fallback ?? key;
            Object.entries(options ?? {}).forEach(([name, replacement]) => {
                value = value.replaceAll(`{{${name}}}`, String(replacement));
            });
            return value;
        },
    }),
}));

const mockUsage = vi.hoisted(() => ({
    value: {} as Record<string, unknown>,
}));

vi.mock('../../context/AccountUsageContext', () => ({
    useAccountUsageContext: () => mockUsage.value,
}));

const baseProvider: AccountUsageProviderInfo = {
    id: 'codex',
    display_name: 'Codex',
    enabled: true,
    show_in_menu_bar: false,
    available: true,
    source: 'auth_file',
    confidence: 'high',
    capabilities: ['account_usage', 'account_quota'],
    credential_requirements: [],
    experimental: false,
    default_refresh_interval_secs: 300,
    refresh_interval_secs: 300,
};

function makeSnapshot(overrides: Partial<AccountUsageSnapshot> = {}): AccountUsageSnapshot {
    return {
        provider_id: 'codex',
        account_key: 'account-1',
        account_label: 'user@example.com',
        plan: 'prolite',
        status: 'ok',
        source: 'auth_file',
        confidence: 'high',
        observed_at: '2026-05-13T00:00:00Z',
        stale: false,
        metrics: [{
            metric_key: 'codex.primary',
            label: '5h window',
            unit: 'percent',
            scope: 'workspace',
            used: 12.5,
            limit: 100,
            remaining: 87.5,
            percentage: 12.5,
            reset_at: '2026-05-13T02:30:00Z',
        }, {
            metric_key: 'codex.secondary',
            label: '7d window',
            unit: 'percent',
            scope: 'workspace',
            used: 45,
            limit: 100,
            remaining: 55,
            percentage: 45,
            reset_at: '2026-05-14T03:45:00Z',
        }],
        ...overrides,
    };
}

describe('AccountUsageCard', () => {
    beforeEach(() => {
        mockUsage.value = {
            snapshots: [makeSnapshot()],
            providers: [baseProvider],
            refreshing: false,
            refreshingProviders: {},
            providerErrors: {},
            refreshAll: vi.fn(),
            refreshProvider: vi.fn(),
            authorizeProvider: vi.fn(),
            authorizingProviders: {},
        };
    });

    it('新版只有一个共享额度进度条，Grok Bot 显示占用明细', () => {
        mockUsage.value.providers = [{ ...baseProvider, id: 'cursor', display_name: 'Cursor', capabilities: ['local_credential_import'] }];
        mockUsage.value.snapshots = [makeSnapshot({ provider_id: 'cursor', plan: 'Ultra', metrics: [
            { metric_key: 'cursor.shared', label: 'Shared usage', unit: 'percent', scope: 'account', percentage: 42, remaining: 58 },
            { metric_key: 'cursor.product.grok_bot', label: 'Grok Bot', unit: 'percent_share', scope: 'account', used: 12.5 },
        ] })];
        const markup = renderToStaticMarkup(<AccountUsageCard />);
        expect(markup.match(/role="progressbar"/g)).toHaveLength(1);
        expect(markup).toContain('Shared plan');
        expect(markup).toContain('58.0% left');
        expect(markup).toContain('12.5% of shared allowance');
        expect(markup).toContain('Included in the shared usage above');
    });

    it('新版缺少 Grok Bot 明细时不显示为零，旧版不展示 Grok Bot', () => {
        mockUsage.value.providers = [{ ...baseProvider, id: 'cursor', display_name: 'Cursor', capabilities: ['local_credential_import'] }];
        mockUsage.value.snapshots = [makeSnapshot({ provider_id: 'cursor', metrics: [
            { metric_key: 'cursor.shared', label: 'Shared usage', unit: 'percent', scope: 'account', percentage: 42 },
            { metric_key: 'cursor.product.grok_bot', label: 'Grok Bot', unit: 'percent_share', scope: 'account' },
        ] })];
        let markup = renderToStaticMarkup(<AccountUsageCard />);
        expect(markup).toContain('Details unavailable');
        expect(markup).not.toContain('0.0% of shared allowance');
        mockUsage.value.snapshots = [makeSnapshot({ provider_id: 'cursor', metrics: [
            { metric_key: 'cursor.auto', label: 'Cursor Models', unit: 'percent', scope: 'account', percentage: 20 },
            { metric_key: 'cursor.api', label: 'Other Models', unit: 'percent', scope: 'account', percentage: 80 },
        ] })];
        markup = renderToStaticMarkup(<AccountUsageCard />);
        expect(markup.match(/role="progressbar"/g)).toHaveLength(2);
        expect(markup).toContain('Legacy plan');
        expect(markup).not.toContain('Grok Bot');
    });

    it('授权过期展示重新授权并隐藏缓存额度，渲染不会发起授权', () => {
        mockUsage.value.providers = [{ ...baseProvider, id: 'cursor', display_name: 'Cursor', capabilities: ['local_credential_import'] }];
        mockUsage.value.snapshots = [makeSnapshot({ provider_id: 'cursor', status: 'stale', stale: true,
            error: { code: 'auth_required', message: '请先在 Cursor 登录' },
        })];
        const markup = renderToStaticMarkup(<AccountUsageCard />);
        expect(markup).toContain('Authorization expired');
        expect(markup).toContain('Reauthorize');
        expect(markup).toContain('请先在 Cursor 登录');
        expect(markup).not.toContain('role="progressbar"');
        expect(mockUsage.value.authorizeProvider).not.toHaveBeenCalled();
    });

    it('渲染正常账号用量指标', () => {
        const markup = renderToStaticMarkup(<AccountUsageCard />);

        expect(markup).toContain('Account Usage');
        expect(markup).toContain('Codex');
        expect(markup).toContain('5x');
        expect(markup).toContain('user@example.com');
        expect(markup).toContain('5h window');
        expect(markup).toContain('12.5%');
        expect(markup).toContain('7d window');
        expect(markup).toContain('45.0%');
        expect(markup).toContain('usage-progress-fill');
        expect(markup).toContain('usage-reset-time');
    });

    it.each(['network', 'rate_limited', 'schema_changed'] as const)('首次刷新 %s 失败只展示错误与重试入口', code => {
        mockUsage.value.providers = [{ ...baseProvider, id: 'cursor', display_name: 'Cursor', capabilities: ['local_credential_import'] }];
        mockUsage.value.snapshots = [makeSnapshot({ provider_id: 'cursor', account_key: 'pending', status: code,
            metrics: [], error: { code, message: '额度暂不可用' },
        })];
        const markup = renderToStaticMarkup(<AccountUsageCard />);
        expect(markup).toContain('额度暂不可用');
        expect(markup).toContain('aria-label="Refresh"');
        expect(markup).not.toContain('No account usage data yet');
        expect(markup).not.toContain('<details');
        expect(markup).not.toContain('Collapse account details');
    });

    it('逐笔额度按绝对到期时间排序并兼容旧缓存', () => {
        const base = { unit: 'reset_credit', label: 'Reset credit', scope: 'workspace' as const, remaining: 1 };
        const metrics = [
            { ...base, metric_key: 'codex.reset_credits.available', remaining: 4 },
            { ...base, metric_key: 'codex.reset_credits.entry.0', reset_at: '2026-10-20T01:00:00Z' },
            { ...base, metric_key: 'codex.reset_credits.entry.1', reset_at: '2026-10-20T08:00:00+08:00' },
            { ...base, metric_key: 'codex.reset_credits.entry.2', reset_at: 'invalid' },
        ];
        expect(getResetCreditEntries(metrics).map(entry => entry.metric_key)).toEqual([
            'codex.reset_credits.entry.1', 'codex.reset_credits.entry.0', 'codex.reset_credits.entry.2',
        ]);
        mockUsage.value.snapshots = [makeSnapshot({ metrics })];
        const markup = renderToStaticMarkup(<AccountUsageCard />);
        expect(markup).toContain('2026-10-20T08:00:00+08:00');
        expect(markup).toContain('2026-10-20T01:00:00Z');
        expect(markup.indexOf('2026-10-20T08:00:00+08:00')).toBeLessThan(markup.indexOf('2026-10-20T01:00:00Z'));
        expect(markup).toContain('Expiration unavailable');
        expect(markup).toContain('1 credit details unavailable');
        mockUsage.value.snapshots = [makeSnapshot({ metrics: [metrics[0]] })];
        expect(renderToStaticMarkup(<AccountUsageCard />)).toContain('4 credit details unavailable');
    });

    it('渲染 stale 状态', () => {
        mockUsage.value.snapshots = [makeSnapshot({ stale: true })];
        const markup = renderToStaticMarkup(<AccountUsageCard />);

        expect(markup).toContain('Stale');
    });

    it('渲染 auth-required 错误状态', () => {
        mockUsage.value.snapshots = [makeSnapshot({
            status: 'auth_required',
            error: { code: 'auth_required', message: '需要登录' },
        })];
        const markup = renderToStaticMarkup(<AccountUsageCard />);

        expect(markup).toContain('需要登录');
    });

    it('启用 Provider 但无快照时显示占位状态', () => {
        mockUsage.value.snapshots = [];
        const markup = renderToStaticMarkup(<AccountUsageCard />);

        expect(markup).toContain('Codex');
        expect(markup).toContain('No account usage data yet');
    });

    it('刷新失败且无快照时显示 Provider 错误', () => {
        mockUsage.value.snapshots = [];
        mockUsage.value.providerErrors = { codex: '未找到 Codex auth 文件' };
        const markup = renderToStaticMarkup(<AccountUsageCard />);

        expect(markup).toContain('未找到 Codex auth 文件');
    });

    it('正确格式化 token、usd 与百分比指标', () => {
        expect(formatAccountUsageMetricValue({
            metric_key: 'tokens',
            label: 'Tokens',
            unit: 'token',
            scope: 'local',
            used: 1200,
        })).toBe('1.2K');
        expect(formatAccountUsageMetricValue({
            metric_key: 'cost',
            label: 'Cost',
            unit: 'usd',
            scope: 'local',
            used: 1.2,
        })).toBe('$1.20');
        expect(formatAccountUsageMetricValue({
            metric_key: 'percent',
            label: 'Percent',
            unit: 'percent',
            scope: 'workspace',
            percentage: 45.5,
        })).toBe('45.5%');
        expect(formatAccountUsageMetricValue({
            metric_key: 'reset-credits',
            label: 'Reset credits',
            unit: 'reset_credit',
            scope: 'workspace',
            remaining: 3,
        })).toBe('3');
        expect(formatAccountUsageMetricValue({
            metric_key: 'null-percent',
            label: 'Null percent',
            unit: 'percent',
            scope: 'workspace',
            used: 12,
            percentage: null,
        })).toBe('12 percent');
    });

    it('映射 Provider 套餐标签', () => {
        expect(getAccountUsagePlanBadge('codex', 'prolite')).toBe('5x');
        expect(getAccountUsagePlanBadge('codex', 'unknown-plan')).toBe('unknown-plan');
        expect(getAccountUsagePlanBadge('codex')).toBeNull();
    });

    it('展示 Codex 可用重置次数和最近到期时间', () => {
        mockUsage.value.snapshots = [makeSnapshot({
            metrics: [
                ...makeSnapshot().metrics,
                {
                    metric_key: 'codex.reset_credits.available',
                    label: 'Reset credits',
                    unit: 'reset_credit',
                    scope: 'workspace',
                    remaining: 3,
                    reset_at: '2099-01-01T00:00:00Z',
                },
            ],
        })];

        const markup = renderToStaticMarkup(<AccountUsageCard />);

        expect(markup).toContain('usage-credit-detail');
        expect(markup).toContain('Available resets');
        expect(markup).toContain('Next credit expires in');
        expect(markup).not.toContain('usage-summary-reset');
    });

    it('格式化重置倒计时不展示秒', () => {
        const now = new Date('2026-05-13T00:00:30Z');

        expect(formatAccountUsageResetTime('2026-05-30T20:50:50Z', now)).toBe('17d20h50m');
        expect(formatAccountUsageResetTime('2026-05-13T02:15:59Z', now)).toBe('2h15m');
        expect(formatAccountUsageResetTime('2026-05-13T00:00:50Z', now)).toBe('0m');
        expect(formatAccountUsageResetTime(null, now)).toBeNull();
    });
});
