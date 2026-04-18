import { motion, useSpring, useTransform } from 'framer-motion';
import { useEffect } from 'react';
import { formatTokenCount } from '../../utils/format';

interface BurgerLayerProps {
    label: string;
    count: number;
    color: string;
    maxCount?: number;
}

const MIN_HEIGHT = 8;
const MAX_HEIGHT = 48;

function BurgerLayer({ label, count, color, maxCount = 1_000_000 }: BurgerLayerProps) {
    const spring = useSpring(0, { stiffness: 120, damping: 20 });
    const height = useTransform(
        spring,
        [0, maxCount],
        [MIN_HEIGHT, MAX_HEIGHT]
    );

    useEffect(() => {
        spring.set(count);
    }, [count, spring]);

    return (
        <motion.div
            className="burger-layer"
            style={{
                height,
                background: color,
                borderRadius: 6,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                padding: '0 12px',
                marginBottom: 2,
                overflow: 'hidden',
                minHeight: MIN_HEIGHT,
            }}
        >
            <span className="layer-label">{label}</span>
            <span className="layer-count">{formatTokenCount(count)}</span>
        </motion.div>
    );
}

export default BurgerLayer;
