import type { ThemeMode } from './types'

export const themeStorageKey = 'lightmonitor.theme'

export function getPreferredTheme(): ThemeMode {
  const stored = localStorage.getItem(themeStorageKey)
  if (stored === 'light' || stored === 'dark') return stored
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
}

export function applyTheme(theme: ThemeMode) {
  document.documentElement.setAttribute('data-theme', theme)
  localStorage.setItem(themeStorageKey, theme)
}

export function toggleTheme(current: ThemeMode): ThemeMode {
  return current === 'dark' ? 'light' : 'dark'
}
