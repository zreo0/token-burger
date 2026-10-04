/**
 * 格式化 token 数量
 *
 * @param count 原始 token 数量
 * @param compact 是否使用最多两位小数并移除尾零的紧凑格式
 * @returns K/M/B 展示文本，默认保留既有精度
 */
export function formatTokenCount(count: number, compact = false): string {
    if (compact && count >= 1_000) {
        const divisor = count >= 1_000_000_000 ? 1_000_000_000 : count >= 1_000_000 ? 1_000_000 : 1_000;
        const unit = divisor === 1_000_000_000 ? 'B' : divisor === 1_000_000 ? 'M' : 'K';
        return `${Number((count / divisor).toFixed(2))}${unit}`;
    }
    if (count >= 1_000_000_000) {
        const value = count / 1_000_000_000;
        const precision = value < 10 ? 3 : value < 100 ? 2 : value < 1_000 ? 1 : 0;
        return `${value.toFixed(precision)}B`;
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
