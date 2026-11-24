import { useTranslation } from 'react-i18next';
import { useAccountUsageContext } from '../../context/AccountUsageContext';
import type { AccountUsageProviderInfo, AccountUsageSnapshot } from '../../types';
import './index.css';

/**
 * 判断快照是否需要重新授权，兼容保留旧额度时的 stale 状态
 */
export function needsAccountAuthorization(snapshot?: AccountUsageSnapshot): boolean {
    const status = snapshot?.error?.code ?? snapshot?.status;
    return status === 'auth_required' || status === 'credential_unavailable';
}

/**
 * 展示可复用的本机授权入口，参数为服务、快照和是否展示已连接状态
 * 只有按钮点击会触发授权，挂载组件与后台刷新均不请求系统交互
 */
export default function AccountAuthorization({ provider, snapshot, showConnected = false }: {
    provider: AccountUsageProviderInfo;
    snapshot?: AccountUsageSnapshot;
    showConnected?: boolean;
}) {
    const { t } = useTranslation();
    const { authorizeProvider, authorizingProviders, providerErrors, refreshingProviders } = useAccountUsageContext();
    const expired = provider.available && needsAccountAuthorization(snapshot);
    const error = providerErrors[provider.id] || (needsAccountAuthorization(snapshot) ? snapshot?.error?.message : undefined);
    const busy = !!authorizingProviders?.[provider.id];
    const connected = provider.available && !expired;
    if (connected && !showConnected && !error) return null;

    return (
        <div className={`account-authorization ${connected && !error ? 'connected' : ''}`} aria-busy={busy}>
            <div className="account-authorization-copy">
                <p className="account-authorization-title" role="status">
                    {busy ? t('usage.authorizing', 'Waiting for authorization…')
                        : expired ? t('usage.authorizationExpired', 'Authorization expired')
                            : connected ? t('usage.localConnected', 'Local account connected')
                                : t('usage.connectLocal', 'Connect your {{provider}} account', { provider: provider.display_name })}
                </p>
                {(!connected || error) && (
                    <p className="account-authorization-help">
                        {expired
                            ? t('usage.localReauthHelp', 'Reauthorize to resume updates. Background refreshes never open permission dialogs.')
                            : t('usage.localAuthHelp', 'Sign in to {{provider}} first, then allow access to its local login. Your credentials are stored securely; background refreshes never open permission dialogs.', { provider: provider.display_name })}
                    </p>
                )}
                {error && <p className="account-authorization-error" role="alert">{error}</p>}
            </div>
            <button type="button" className="account-authorization-button"
                disabled={busy || !!refreshingProviders[provider.id]}
                onClick={() => { void authorizeProvider(provider.id); }}>
                {busy ? t('usage.authorizingShort', 'Authorizing…')
                    : provider.available ? t('usage.reauthorize', 'Reauthorize')
                        : t('usage.authorizeLocal', 'Authorize access')}
            </button>
        </div>
    );
}
