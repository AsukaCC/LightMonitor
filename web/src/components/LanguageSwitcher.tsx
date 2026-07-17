import { Languages } from 'lucide-react'
import { useI18n } from '../i18n'
import type { Language } from '../i18n'

export function LanguageSwitcher() {
  const { language, setLanguage, t } = useI18n()
  return (
    <div className="language-switcher" title={t('切换语言')}>
      <Languages size={16} aria-hidden />
      <select
        aria-label={t('切换语言')}
        value={language}
        onChange={(event) => setLanguage(event.target.value as Language)}
      >
        <option value="zh-CN">中文</option>
        <option value="ja-JP">日本語</option>
        <option value="en-US">English</option>
      </select>
    </div>
  )
}
