import { renderToStaticMarkup } from 'react-dom/server';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import AccountUsageCard, { formatAccountUsageMetricValue, formatAccountUsageResetTime, getAccountUsagePlanBadge } from '../AccountUsageCard';
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
        };
    });

    it('渲染正常账号用量指标', () => {
        const markup = renderToStaticMarkup(<AccountUsageCard />);

        expect(markup).toContain('Account Usage');
        expect(markup).toContain('Codex');
        expect(markup).toContain('5x');
        expect(markup).not.toContain('user@example.com');
        expect(markup).toContain('5h window');
        expect(markup).toContain('12.5%');
        expect(markup).toContain('7d window');
        expect(markup).toContain('45.0%');
        expect(markup).toContain('usage-progress-fill');
        expect(markup).toContain('usage-reset-time');
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

        expect(markup).toContain('usage-reset-credit-badge');
        expect(markup).toContain('reset 3 ·');
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
