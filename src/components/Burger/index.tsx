import { useTranslation } from 'react-i18next';
import BurgerLayer from './BurgerLayer';
import { getThemeById } from './themes';
import type { TimeRange, TokenSummary } from '../../types';
import './index.css';

interface BurgerProps {
    /** 当前用量汇总 */
    summary: TokenSummary | null;
    /** 统计范围 */
    range: TimeRange;
    /** 已选食材主题 */
    themeId?: string;
}

/**
 * 将四类用量组合为完整汉堡，参数为汇总与主题，返回分层统计视图
 */
function Burger({ summary, range, themeId }: BurgerProps) {
    const { t } = useTranslation();
    const { colors } = getThemeById(themeId ?? 'warm');
    const total = summary ? summary.input + summary.cache_create + summary.cache_read + summary.output : 0;
    return (
        <div className={`burger-stack${total === 0 ? ' burger-empty' : ''}`}>
            <BurgerLayer label={t('popup.output')} count={summary?.output ?? 0} color={colors.output} variant="bread" position="top" total={total} range={range} />
            <BurgerLayer label={t('popup.cache_read')} count={summary?.cache_read ?? 0} color={colors.cache_read} variant="cache" position="middle" total={total} range={range} />
            <BurgerLayer label={t('popup.cache_create')} count={summary?.cache_create ?? 0} color={colors.cache_create} variant="cache" position="middle" total={total} range={range} />
            <BurgerLayer label={t('popup.input')} count={summary?.input ?? 0} color={colors.input} variant="bread" position="bottom" total={total} range={range} />
        </div>
    );
}

export default Burger;
