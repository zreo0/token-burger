import { createContext, useContext, type ReactNode } from 'react';
import { useTokenStream } from '../hooks/useTokenStream';
import type { TokenSummary, TimeRange } from '../types';

interface TokenContextValue {
    summary: TokenSummary | null;
    loading: boolean;
    error: string | null;
    refresh: () => Promise<void>;
    range: TimeRange;
    setRange: (range: TimeRange) => void;
}

const TokenContext = createContext<TokenContextValue | null>(null);

export function TokenProvider({ children }: { children: ReactNode }) {
    const value = useTokenStream();
    return (
        <TokenContext.Provider value={value}>
            {children}
        </TokenContext.Provider>
    );
}

export function useToken() {
    const ctx = useContext(TokenContext);
    if (!ctx) {
        throw new Error('useToken must be used within TokenProvider');
    }
    return ctx;
}
