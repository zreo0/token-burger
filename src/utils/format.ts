/**
 * token 数格式化（K/M/B 规则，保留一位小数）
 */
export function formatTokenCount(count: number): string {
    if (count >= 1_000_000_000) {
        return `${(count / 1_000_000_000).toFixed(1)}B`;
    }
    if (count >= 1_000_000) {
        return `${(count / 1_000_000).toFixed(1)}M`;
    }
    if (count >= 1_000) {
        return `${(count / 1_000).toFixed(1)}K`;
    }
    return count.toString();
}

/**
 * 金额格式化（$X.XX）
 */
export function formatCost(cost: number): string {
    if (cost < 0.01 && cost > 0) {
        return '<$0.01';
    }
    return `$${cost.toFixed(2)}`;
}
