import type { ModelPrice, PricingTable, TokenBreakdown } from '../types';

// 仅用于计费匹配：保留数据库中的原始 model_id，价格查询时再映射到官方/价格表模型名。
export const MODEL_PRICE_ALIASES: Record<string, string> = {
    'gpt-5.5-fast': 'gpt-5.5',
};

export function normalizeModelIdForPricing(modelId: string): string {
    return MODEL_PRICE_ALIASES[modelId] ?? modelId;
}

function stripProviderPrefix(modelId: string): string {
    return modelId.replace(/^[^/]+\//, '');
}

function stripModelSuffix(modelId: string): string {
    return modelId
        .replace(/-\d{8}$/, '')
        .replace(/-v\d+$/, '');
}

function getPricingCandidates(modelId: string): string[] {
    const withoutPrefix = stripProviderPrefix(modelId);
    const withoutSuffix = stripModelSuffix(modelId);
    const withoutPrefixAndSuffix = stripModelSuffix(withoutPrefix);
    // 匹配顺序保持保守：原始值优先，其次别名，再尝试去 provider 前缀和版本/日期后缀。
    const candidates = [
        modelId,
        normalizeModelIdForPricing(modelId),
        withoutPrefix,
        normalizeModelIdForPricing(withoutPrefix),
        withoutSuffix,
        normalizeModelIdForPricing(withoutSuffix),
        withoutPrefixAndSuffix,
        normalizeModelIdForPricing(withoutPrefixAndSuffix),
    ];

    return [...new Set(candidates)];
}

/**
 * 模型名匹配：精确 → 别名 → Provider 前缀 → 后缀归一化 → $0
 */
export function matchModelPrice(
    modelId: string,
    pricing: PricingTable
): ModelPrice | null {
    for (const candidate of getPricingCandidates(modelId)) {
        if (pricing[candidate]) {
            return pricing[candidate];
        }
    }

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
