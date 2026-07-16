import { Moon, Sun } from 'lucide-react'
import type { ThemeMode } from '../types'
import { useI18n } from '../i18n'

export function ThemeToggle({
  theme,
  onToggle,
}: {
  theme: ThemeMode
  onToggle: () => void
}) {
  const { t } = useI18n()
  return (
    <button className="icon-btn" onClick={onToggle} title={theme === 'dark' ? t('浅色') : t('深色')} type="button">
      {theme === 'dark' ? <Sun size={18} /> : <Moon size={18} />}
    </button>
  )
}
