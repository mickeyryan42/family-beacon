/**
 * Calm, high-contrast theme. Dark by default: easier on the eyes at night and
 * cheaper on OLED batteries during an outage.
 */
import React, {createContext, useContext, useMemo, useState} from 'react';
import {useColorScheme} from 'react-native';

import type {CheckInState} from '../sync/protocol';

export type ThemeMode = 'dark' | 'light' | 'system';

export interface Palette {
  background: string;
  surface: string;
  surfaceRaised: string;
  border: string;
  text: string;
  textMuted: string;
  accent: string;
  onAccent: string;
  danger: string;
  bannerOffline: string;
  bannerOnline: string;
  status: Record<CheckInState | 'unknown', string>;
}

const dark: Palette = {
  background: '#0F1418',
  surface: '#182027',
  surfaceRaised: '#202A33',
  border: '#2C3843',
  text: '#ECF1F4',
  textMuted: '#9AA8B3',
  accent: '#F2B84B',
  onAccent: '#1A1408',
  danger: '#EE7B6B',
  bannerOffline: '#2B2414',
  bannerOnline: '#16241D',
  status: {
    safe: '#63C38F',
    on_my_way: '#6CAEEB',
    at_rendezvous: '#B194E6',
    need_help: '#EE7B6B',
    unknown: '#6F7D88',
  },
};

const light: Palette = {
  background: '#F6F4EF',
  surface: '#FFFFFF',
  surfaceRaised: '#FBFAF7',
  border: '#E1DDD4',
  text: '#1C2329',
  textMuted: '#5C6872',
  accent: '#B9800F',
  onAccent: '#FFFFFF',
  danger: '#C2412F',
  bannerOffline: '#FBEFD3',
  bannerOnline: '#E3F2E8',
  status: {
    safe: '#2E8B57',
    on_my_way: '#2F74B5',
    at_rendezvous: '#7550B8',
    need_help: '#C2412F',
    unknown: '#8A949C',
  },
};

export const spacing = {xs: 4, sm: 8, md: 16, lg: 24, xl: 32};
export const radius = {md: 12, lg: 18};
/** Minimum tap target. Larger than platform minimums on purpose. */
export const TAP_TARGET = 56;

export const typography = {
  title: {fontSize: 28, fontWeight: '700' as const},
  heading: {fontSize: 20, fontWeight: '600' as const},
  body: {fontSize: 17, lineHeight: 24},
  small: {fontSize: 15, lineHeight: 20},
};

export interface Theme {
  mode: ThemeMode;
  isDark: boolean;
  colors: Palette;
  setMode(mode: ThemeMode): void;
}

const ThemeContext = createContext<Theme | null>(null);

export function ThemeProvider({children}: {children: React.ReactNode}) {
  const [mode, setMode] = useState<ThemeMode>('dark');
  const system = useColorScheme();
  const isDark = mode === 'system' ? system !== 'light' : mode === 'dark';

  const value = useMemo<Theme>(
    () => ({mode, isDark, colors: isDark ? dark : light, setMode}),
    [mode, isDark],
  );
  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): Theme {
  const theme = useContext(ThemeContext);
  if (!theme) {
    throw new Error('useTheme must be used inside <ThemeProvider>');
  }
  return theme;
}
