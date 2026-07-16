import type { ReactNode } from 'react'
import { clamp } from '../utils'
import { useI18n } from '../i18n'

export function MetricBar({
  icon,
  label,
  value,
  suffix = '%',
  tone = 'cpu',
}: {
  icon?: ReactNode
  label: string
  value: number
  suffix?: string
  tone?: 'cpu' | 'mem' | 'disk'
}) {
  const { t } = useI18n()
  const normalized = clamp(value)
  return (
    <div className={`metric-bar ${tone}`}>
      <div className="metric-head">
        {icon}
        <span>{label}</span>
        <strong className="metric-percent">
          {t('占比')}&nbsp;
          {normalized.toFixed(2)}
          {suffix}
        </strong>
      </div>
      <div className="bar-track">
        <i style={{ width: `${normalized}%` }} />
      </div>
    </div>
  )
}
