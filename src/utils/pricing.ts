import type { ModelPrice, PricingTable, TokenBreakdown } from '../types';

/**
 * 模型名匹配：精确 → 归一化 → Provider 前缀 → $0
 */
export function matchModelPrice(
    modelId: string,
    pricing: PricingTable
): ModelPrice | null {
    // 1. 精确匹配
    if (pricing[modelId]) {
        return pricing[modelId];
    }

    // 2. 归一化匹配（去日期后缀 -YYYYMMDD、版本号 -v\d）
    const normalized = modelId
        .replace(/-\d{8}$/, '')
        .replace(/-v\d+$/, '');
    if (pricing[normalized]) {
        return pricing[normalized];
    }

    // 3. Provider 前缀匹配（去 anthropic/ 等前缀）
    const withoutPrefix = modelId.replace(/^[^/]+\//, '');
    if (pricing[withoutPrefix]) {
        return pricing[withoutPrefix];
    }

    // 4. 未匹配
    return null;
}

/**
 * 计算单个模型的花费
 */
export function calculateModelCost(
    breakdown: TokenBreakdown,
    price: ModelPrice
): number {
    return (
        breakdown.input * price.input_cost_per_token +
        breakdown.output * price.output_cost_per_token +
        breakdown.cache_read * price.cache_read_input_token_cost +
        breakdown.cache_create * price.cache_creation_input_token_cost
    );
}

/**
 * 计算全部模型的总花费
 */
export function calculateTotalCost(
    byModel: Record<string, TokenBreakdown>,
    pricing: PricingTable,
    agentCost = 0
): number {
    let total = agentCost;
    for (const [modelId, breakdown] of Object.entries(byModel)) {
        if (breakdown.agent_cost > 0) {
            total += breakdown.agent_cost;
            continue;
        }

        const price = matchModelPrice(modelId, pricing);
        if (price) {
            total += calculateModelCost(breakdown, price);
        }
    }
    return total;
}
