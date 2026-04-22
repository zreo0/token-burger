import { motion, useSpring, useTransform } from 'framer-motion';
import { useEffect, useState } from 'react';
import { formatTokenCount } from '../../utils/format';
import type { TimeRange } from '../../types';

interface BurgerLayerProps {
    label: string;
    count: number;
    color: string;
    variant: 'bread' | 'cache';
    position: 'top' | 'middle' | 'bottom';
    maxCount?: number;
    range?: TimeRange;
}

export const BREAD_HEIGHT = 36;
export const CACHE_MIN_HEIGHT = 24;
export const CACHE_MAX_HEIGHT = 72;

export function getLayerSpringConfig(range: TimeRange) {
    if (range === 'today') {
        return { stiffness: 110, damping: 24, mass: 0.9 };
    }

    return { stiffness: 320, damping: 34, mass: 0.8 };
}

export function getCacheLayerHeight(count: number, maxCount: number): number {
    if (count <= 0 || maxCount <= 0) {
        return CACHE_MIN_HEIGHT;
    }

    const normalized = Math.min(count / maxCount, 1);
    const curved = Math.pow(normalized, 0.45);

    return CACHE_MIN_HEIGHT + (CACHE_MAX_HEIGHT - CACHE_MIN_HEIGHT) * curved;
}

function BurgerLayer({
    label,
    count,
    color,
    variant,
    position: _position,
    maxCount = 1_000_000,
    range = 'today',
}: BurgerLayerProps) {
    const spring = useSpring(count, getLayerSpringConfig(range));
    const height = useTransform(
        spring,
        (latest) => (variant === 'bread' ? BREAD_HEIGHT : getCacheLayerHeight(latest, maxCount))
    );

    useEffect(() => {
        spring.set(count);
    }, [count, spring]);

    const [displayCount, setDisplayCount] = useState(count);

    useEffect(() => {
        const unsubscribe = spring.on('change', (latest) => {
            setDisplayCount(Math.round(latest));
        });

        return unsubscribe;
    }, [spring]);

    return (
        <motion.div
            className={`burger-layer burger-layer--${variant}`}
            layout
            style={{
                height,
                backgroundColor: color,
            }}
            transition={{
                layout: range === 'today'
                    ? { duration: 0.28, ease: 'easeOut' }
                    : { duration: 0.18, ease: 'easeOut' },
            }}
            aria-label={`${label} ${formatTokenCount(displayCount)}`}
        >
            <span className="layer-label">{label}</span>
            <span className="layer-count">{formatTokenCount(displayCount)}</span>
        </motion.div>
    );
}

export default BurgerLayer;
