import { createContext, useContext, ReactNode } from 'react';
import { useAccountUsage } from '../hooks/useAccountUsage';

export type AccountUsageContextType = ReturnType<typeof useAccountUsage>;

const AccountUsageContext = createContext<AccountUsageContextType | undefined>(undefined);

export function AccountUsageProvider({ children }: { children: ReactNode }) {
    const accountUsage = useAccountUsage();

    return (
        <AccountUsageContext.Provider value={accountUsage}>
            {children}
        </AccountUsageContext.Provider>
    );
}

export function useAccountUsageContext() {
    const context = useContext(AccountUsageContext);
    if (!context) {
        throw new Error('useAccountUsageContext must be used within an AccountUsageProvider');
    }
    return context;
}
