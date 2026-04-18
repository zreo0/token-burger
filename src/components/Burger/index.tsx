import { useTranslation } from 'react-i18next';
import BurgerLayer from './BurgerLayer';
import type { TokenSummary } from '../../types';
import './index.css';

interface BurgerProps {
    summary: TokenSummary | null;
}

const LAYER_COLORS = {
    input: 'linear-gradient(135deg, #60a5fa, #3b82f6)',
    cache_create: 'linear-gradient(135deg, #a78bfa, #8b5cf6)',
    cache_read: 'linear-gradient(135deg, #34d399, #10b981)',
    output: 'linear-gradient(135deg, #fb923c, #f97316)',
};

function Burger({ summary }: BurgerProps) {
    const { t } = useTranslation();

    const maxCount = summary
        ? Math.max(summary.input, summary.cache_create, summary.cache_read, summary.output, 1)
        : 1;

    return (
        <div className="burger-container">
            {/* 顶部面包 */}
            <div className="burger-bun burger-bun-top" />

            <div className="burger-layers">
                <BurgerLayer
                    label={t('popup.output')}
                    count={summary?.output ?? 0}
                    color={LAYER_COLORS.output}
                    maxCount={maxCount}
                />
                <BurgerLayer
                    label={t('popup.cache_read')}
                    count={summary?.cache_read ?? 0}
                    color={LAYER_COLORS.cache_read}
                    maxCount={maxCount}
                />
                <BurgerLayer
                    label={t('popup.cache_create')}
                    count={summary?.cache_create ?? 0}
                    color={LAYER_COLORS.cache_create}
                    maxCount={maxCount}
                />
                <BurgerLayer
                    label={t('popup.input')}
                    count={summary?.input ?? 0}
                    color={LAYER_COLORS.input}
                    maxCount={maxCount}
                />
            </div>

            {/* 底部面包 */}
            <div className="burger-bun burger-bun-bottom" />
        </div>
    );
}

export default Burger;
