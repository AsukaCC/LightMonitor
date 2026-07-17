import { currentLocale, translate } from './i18n'

export function percent(used: number, total: number) {
  return total > 0 ? (used / total) * 100 : 0
}

export function formatCpuUsage(cpuPercent: number, cpuCores: number) {
  if (!Number.isFinite(cpuCores) || cpuCores <= 0) return 'CPU'
  const usedCores = (cpuPercent / 100) * cpuCores
  return translate('CPU 已用 {used} / {total} 核', {
    used: usedCores.toFixed(2), total: cpuCores.toFixed(2),
  })
}

export function formatBytes(value: number) {
  if (!Number.isFinite(value) || value <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const unitIndex = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1)
  return `${(value / 1024 ** unitIndex).toFixed(unitIndex === 0 ? 0 : 2)} ${units[unitIndex]}`
}

export function formatResourceUsage(label: string, used: number, total: number) {
  return translate('{label} 已用 {used} / {total}', {
    label: translate(label), used: formatBytes(used), total: formatBytes(total),
  })
}

export function formatDuration(seconds: number) {
  const days = Math.floor(seconds / 86400)
  const hours = Math.floor((seconds % 86400) / 3600)
  const mins = Math.floor((seconds % 3600) / 60)
  if (days > 0) return translate('{days}d {hours}h', { days, hours })
  if (hours > 0) return translate('{hours}h {minutes}m', { hours, minutes: mins })
  return translate('{minutes}m', { minutes: mins })
}

export function formatLoad(load: [number, number, number]) {
  return load.map((v) => v.toFixed(2)).join(' / ')
}

export async function readError(response: Response) {
  try {
    const body = (await response.json()) as { error?: string }
    return body.error ?? response.statusText
  } catch {
    return response.statusText
  }
}

export function clamp(value: number, min = 0, max = 100) {
  return Math.max(min, Math.min(max, value))
}

export function formatRelativeTime(iso?: string) {
  if (!iso) return translate('从未上报')
  const ts = new Date(iso).getTime()
  if (Number.isNaN(ts)) return translate('未知')
  const diff = Math.max(0, Date.now() - ts)
  const sec = Math.floor(diff / 1000)
  if (sec < 15) return translate('刚刚')
  if (sec < 60) return translate('{count} 秒前', { count: sec })
  const min = Math.floor(sec / 60)
  if (min < 60) return translate('{count} 分钟前', { count: min })
  const hour = Math.floor(min / 60)
  if (hour < 48) return translate('{count} 小时前', { count: hour })
  const day = Math.floor(hour / 24)
  if (day < 14) return translate('{count} 天前', { count: day })
  return new Date(iso).toLocaleString(currentLocale())
}

export function isStaleHost(status: string, hasAgent: boolean) {
  return !hasAgent || status === 'offline' || status === 'error' || status === 'pending'
}
