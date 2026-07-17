import { Check, ChevronDown, Languages } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useI18n } from '../i18n'
import type { Language } from '../i18n'

const languageOptions: Array<{ value: Language; label: string }> = [
  { value: 'zh-CN', label: '中文' },
  { value: 'ja-JP', label: '日本語' },
  { value: 'en-US', label: 'English' },
]

export function LanguageSwitcher() {
  const { language, setLanguage, t } = useI18n()
  const [open, setOpen] = useState(false)
  const switcherRef = useRef<HTMLDivElement>(null)
  const currentOption = languageOptions.find((option) => option.value === language) ?? languageOptions[0]

  useEffect(() => {
    if (!open) return

    function handlePointerDown(event: PointerEvent) {
      if (event.target instanceof Node && !switcherRef.current?.contains(event.target)) {
        setOpen(false)
      }
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') setOpen(false)
    }

    document.addEventListener('pointerdown', handlePointerDown)
    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown)
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [open])

  return (
    <div className="language-switcher" data-open={open} ref={switcherRef} title={t('切换语言')}>
      <button
        aria-expanded={open}
        aria-haspopup="listbox"
        aria-label={t('切换语言')}
        className="language-trigger"
        onClick={() => setOpen((current) => !current)}
        type="button"
      >
        <Languages size={16} aria-hidden />
        <span className="language-value">{currentOption.label}</span>
        <ChevronDown className="language-chevron" size={14} aria-hidden />
      </button>

      {open && (
        <div aria-label={t('切换语言')} className="language-menu" role="listbox">
          {languageOptions.map((option) => (
            <button
              aria-selected={option.value === language}
              className="language-option"
              key={option.value}
              onClick={() => {
                setLanguage(option.value)
                setOpen(false)
              }}
              role="option"
              type="button"
            >
              <span>{option.label}</span>
              {option.value === language && <Check className="language-check" size={15} aria-hidden />}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}
