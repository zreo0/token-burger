import { motion, useReducedMotion, useSpring } from 'framer-motion';
import { useEffect, useState } from 'react';
import { formatTokenCount } from '../../utils/format';
import type { TimeRange } from '../../types';

interface BurgerLayerProps {
    /** 类型名称 */
    label: string;
    /** 当前用量 */
    count: number;
    /** 食材颜色 */
    color: string;
    /** 食材类型 */
    variant: 'bread' | 'cache';
    /** 汉堡中的位置 */
    position: 'top' | 'middle' | 'bottom';
    /** 总用量，用于展示准确占比 */
    total: number;
    /** 当前时间范围 */
    range?: TimeRange;
}

/**
 * 按时间范围返回数值动画参数，实时更新较柔和，范围切换较快
 */
export function getLayerSpringConfig(range: TimeRange) {
    return range === 'today'
        ? { stiffness: 110, damping: 24, mass: 0.9 }
        : { stiffness: 320, damping: 34, mass: 0.8 };
}

/**
 * 根据用量和主题渲染一层食材，固定高度避免将装饰厚度误读为占比
 */
function BurgerLayer({ label, count, color, variant, position, total, range = 'today' }: BurgerLayerProps) {
    const spring = useSpring(count, getLayerSpringConfig(range));
    const reducedMotion = useReducedMotion();
    const [displayCount, setDisplayCount] = useState(count);

    useEffect(() => { spring.set(count); }, [count, spring]);
    useEffect(() => spring.on('change', latest => setDisplayCount(Math.round(latest))), [spring]);

    return (
        <motion.div className={`burger-layer burger-layer--${variant} burger-layer--${position}`}
            style={{ backgroundColor: color }} aria-label={`${label} ${count.toLocaleString()}`}>
            {position === 'top' && <span className="sesame" aria-hidden="true">{Array.from({ length: 9 }, (_, index) => <i key={index} />)}</span>}
            <span className="layer-label">{label}</span>
            <span className="layer-count" title={count.toLocaleString()}>{formatTokenCount(reducedMotion ? count : displayCount, true)}</span>
            <span className="layer-percent">{total > 0 ? `${Math.round(count / total * 100)}%` : '—'}</span>
        </motion.div>
    );
}

export default BurgerLayer;
