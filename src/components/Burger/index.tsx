import { useTranslation } from 'react-i18next';
import { LayoutGroup, motion } from 'framer-motion';
import BurgerLayer from './BurgerLayer';
import type { TimeRange, TokenSummary } from '../../types';
import './index.css';

interface BurgerProps {
    summary: TokenSummary | null;
    range: TimeRange;
}

const LAYER_COLORS = {
    output: '#F8D08E',
    cache_read: '#B2DE75',
    cache_create: '#F4B298',
    input: '#F8C97E',
};

function Burger({ summary, range }: BurgerProps) {
    const { t } = useTranslation();

    const maxCount = summary
        ? Math.max(summary.cache_create, summary.cache_read, 1)
        : 1;

    const total = summary
        ? summary.input + summary.cache_create + summary.cache_read + summary.output
        : 0;

    return (
        <LayoutGroup>
            <motion.div className="burger-container" layout>
                <motion.div className="burger-stack" layout>
                    <BurgerLayer
                        label={t('popup.input')}
                        count={summary?.input ?? 0}
                        color={LAYER_COLORS.input}
                        variant="bread"
                        position="bottom"
                        maxCount={maxCount}
                        range={range}
                    />
                    <BurgerLayer
                        label={t('popup.cache_create')}
                        count={summary?.cache_create ?? 0}
                        color={LAYER_COLORS.cache_create}
                        variant="cache"
                        position="middle"
                        maxCount={maxCount}
                        range={range}
                    />
                    <BurgerLayer
                        label={t('popup.cache_read')}
                        count={summary?.cache_read ?? 0}
                        color={LAYER_COLORS.cache_read}
                        variant="cache"
                        position="middle"
                        maxCount={maxCount}
                        range={range}
                    />
                    <BurgerLayer
                        label={t('popup.output')}
                        count={summary?.output ?? 0}
                        color={LAYER_COLORS.output}
                        variant="bread"
                        position="top"
                        maxCount={maxCount}
                        range={range}
                    />
                </motion.div>

                {/* 精致的占比进度条 */}
                {total > 0 && (
                    <motion.div 
                        className="burger-progress-bar"
                        layout
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                    >
                        {(summary?.input ?? 0) > 0 && (
                            <div className="progress-segment" style={{ width: `${((summary?.input ?? 0) / total) * 100}%`, backgroundColor: LAYER_COLORS.input }} />
                        )}
                        {(summary?.cache_create ?? 0) > 0 && (
                            <div className="progress-segment" style={{ width: `${((summary?.cache_create ?? 0) / total) * 100}%`, backgroundColor: LAYER_COLORS.cache_create }} />
                        )}
                        {(summary?.cache_read ?? 0) > 0 && (
                            <div className="progress-segment" style={{ width: `${((summary?.cache_read ?? 0) / total) * 100}%`, backgroundColor: LAYER_COLORS.cache_read }} />
                        )}
                        {(summary?.output ?? 0) > 0 && (
                            <div className="progress-segment" style={{ width: `${((summary?.output ?? 0) / total) * 100}%`, backgroundColor: LAYER_COLORS.output }} />
                        )}
                    </motion.div>
                )}
            </motion.div>
        </LayoutGroup>
    );
}

export default Burger;
