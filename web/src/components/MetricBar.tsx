import type { ReactNode } from 'react'
import { clamp } from '../utils'

export function MetricBar({
  icon,
  name,
  detail,
  value,
  suffix = '%',
  tone = 'cpu',
}: {
  icon?: ReactNode
  name: string
  detail?: string
  value: number
  suffix?: string
  tone?: 'cpu' | 'mem' | 'disk'
}) {
  const normalized = clamp(value)

  return (
    <div className={`metric-bar ${tone}`}>
      <div className="metric-head">
        <div className="metric-name">
          {icon}
          <span>{name}</span>
        </div>
        <strong className="metric-percent">
          {normalized.toFixed(2)}
          {suffix}
        </strong>
      </div>
      {detail ? <p className="metric-detail">{detail}</p> : <p className="metric-detail metric-detail-empty">&nbsp;</p>}
      <div className="bar-track">
        <i style={{ width: `${normalized}%` }} />
      </div>
    </div>
  )
}
