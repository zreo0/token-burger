// 汉堡配色主题
export interface LayerColors {
    output: string;
    cache_read: string;
    cache_create: string;
    input: string;
}

export interface BurgerTheme {
    id: string;
    labelKey: string;
    colors: LayerColors;
}

export const BURGER_THEMES: BurgerTheme[] = [
    {
        id: 'warm',
        labelKey: 'settings.themeWarm',
        colors: {
            output: '#E5B360',
            cache_read: '#97B95E',
            cache_create: '#DF7159',
            input: '#E0B36B',
        },
    },
    {
        id: 'muted',
        labelKey: 'settings.themeMuted',
        colors: {
            output: '#D9C4A9',
            cache_read: '#8BA382',
            cache_create: '#B86B5A',
            input: '#DAC4A7',
        },
    },
    {
        id: 'vivid',
        labelKey: 'settings.themeVivid',
        colors: {
            output: '#E5B567',
            cache_read: '#20C997',
            cache_create: '#D84A79',
            input: '#E5B567',
        },
    },
];

export const DEFAULT_THEME_ID = 'warm';

export function getThemeById(id: string): BurgerTheme {
    return BURGER_THEMES.find((t) => t.id === id) ?? BURGER_THEMES[0];
}
