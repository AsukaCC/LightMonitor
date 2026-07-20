import {
  Activity,
  Cpu,
  Download,
  Globe2,
  HardDrive,
  MapPin,
  MemoryStick,
  Moon,
  RefreshCw,
  Server,
  ShieldCheck,
  Sun,
  Upload,
  Wifi,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { fetchPublicHosts } from '../api'
import { MetricBar } from '../components/MetricBar'
import { LanguageSwitcher } from '../components/LanguageSwitcher'
import { useI18n } from '../i18n'
import type { PublicHost, ThemeMode } from '../types'
import { statusLabel } from '../types'
import {
  daysUntil,
  formatCpuDetail,
  formatDate,
  formatDuration,
  formatLatency,
  formatLoad,
  formatNetworkRate,
  formatPacketLoss,
  formatUsageDetail,
} from '../utils'

type HostFilter = 'all' | 'online' | 'offline'

export function PublicPage({
  theme,
  onToggleTheme,
}: {
  theme: ThemeMode
  onToggleTheme: () => void
}) {
  const { language, t } = useI18n()
  const [hosts, setHosts] = useState<PublicHost[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const [updatedAt, setUpdatedAt] = useState<Date>()
  const [hostFilter, setHostFilter] = useState<HostFilter>('all')

  const load = useCallback(async () => {
    try {
      const data = await fetchPublicHosts()
      setHosts(data)
      setError('')
      setUpdatedAt(new Date())
    } catch (err) {
      setError(err instanceof Error ? err.message : t('加载失败'))
    } finally {
      setLoading(false)
    }
  }, [t])

  useEffect(() => {
    void load()
    const timer = window.setInterval(() => void load(), 5000)
    return () => window.clearInterval(timer)
  }, [load])

  const summary = useMemo(
    () => ({
      total: hosts.length,
      online: hosts.filter((h) => h.status === 'online').length,
      offline: hosts.filter((h) => h.status === 'offline' || h.status === 'error').length,
    }),
    [hosts],
  )

  const filteredHosts = useMemo(() => {
    if (hostFilter === 'online') {
      return hosts.filter((host) => host.status === 'online')
    }
    if (hostFilter === 'offline') {
      return hosts.filter((host) => host.status === 'offline' || host.status === 'error')
    }
    return hosts
  }, [hostFilter, hosts])

  return (
    <div className="public-page">
      <header className="public-header">
        <div className="brand">
          <div className="brand-mark">
            <Activity size={18} />
          </div>
          <div className="brand-copy">
            <h1>LightMonitor</h1>
            <span>{t('公开监控')}</span>
          </div>
        </div>
        <div className="public-header-actions">
          {updatedAt && (
            <span className="public-header-meta muted small">
              {t('更新于 {time}', { time: updatedAt.toLocaleTimeString(language) })}
            </span>
          )}
          <div className="public-header-tools">
            <LanguageSwitcher />
            <button className="icon-btn" onClick={() => void load()} title={t('刷新')} type="button">
              <RefreshCw size={16} />
            </button>
            <button className="icon-btn" onClick={onToggleTheme} title={t('切换主题')} type="button">
              {theme === 'dark' ? <Sun size={16} /> : <Moon size={16} />}
            </button>
          </div>
          <a className="btn ghost public-header-admin" href="/admin">
            {t('管理入口')}
          </a>
        </div>
      </header>

      <section aria-label={t('按主机状态筛选')} className="summary-row">
        <button
          aria-pressed={hostFilter === 'all'}
          className={`summary-card summary-filter${hostFilter === 'all' ? ' active' : ''}`}
          onClick={() => setHostFilter('all')}
          type="button"
        >
          <div className="summary-icon-wrap"><Server size={18} /></div>
          <div className="summary-info">
            <span>{t('全部')}</span>
            <strong>{summary.total}</strong>
          </div>
        </button>
        <button
          aria-pressed={hostFilter === 'online'}
          className={`summary-card summary-filter online${hostFilter === 'online' ? ' active' : ''}`}
          onClick={() => setHostFilter('online')}
          type="button"
        >
          <div className="summary-icon-wrap"><Wifi size={18} /></div>
          <div className="summary-info">
            <span>{t('在线')}</span>
            <strong>{summary.online}</strong>
          </div>
        </button>
        <button
          aria-pressed={hostFilter === 'offline'}
          className={`summary-card summary-filter offline${hostFilter === 'offline' ? ' active' : ''}`}
          onClick={() => setHostFilter('offline')}
          type="button"
        >
          <div className="summary-icon-wrap"><Activity size={18} /></div>
          <div className="summary-info">
            <span>{t('离线/异常')}</span>
            <strong>{summary.offline}</strong>
          </div>
        </button>
      </section>

      {error && <div className="banner error">{error}</div>}
      {loading && hosts.length === 0 && <div className="empty-state">{t('加载中…')}</div>}
      {!loading && hosts.length === 0 && !error && (
        <div className="empty-state">{t('暂无公开服务器')}</div>
      )}
      {!loading && hosts.length > 0 && filteredHosts.length === 0 && !error && (
        <div className="empty-state">{t('当前筛选下暂无主机')}</div>
      )}

      <section className="public-grid">
        {filteredHosts.map((host) => (
          <article className="public-card" key={host.id}>
            <div className="public-card-head">
              <div style={{ display: 'flex', alignItems: 'center', gap: '10px' }}>
                <span className={`dot ${host.status}`} />
                <div>
                  <h3>{host.name}</h3>
                  <p className="region-line" style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
                    <MapPin size={12} />
                    {host.region || t('未设置地区')}
                  </p>
                </div>
              </div>
              <span className={`status-pill ${host.status}`}>{statusLabel(host.status)}</span>
            </div>

            {host.metrics ? (
              <div className="public-metrics">
                <MetricBar
                  icon={<Cpu size={16} />}
                  name="CPU"
                  detail={formatCpuDetail(host.metrics.cpu_percent, host.metrics.cpu_cores)}
                  value={host.metrics.cpu_percent}
                  tone="cpu"
                />
                <MetricBar
                  icon={<MemoryStick size={16} />}
                  name={t('内存')}
                  detail={formatUsageDetail(
                    host.metrics.memory_used_bytes,
                    host.metrics.memory_total_bytes,
                  )}
                  value={host.metrics.memory_percent}
                  tone="mem"
                />
                <MetricBar
                  icon={<HardDrive size={16} />}
                  name={t('磁盘')}
                  detail={formatUsageDetail(host.metrics.disk_used_bytes, host.metrics.disk_total_bytes)}
                  value={host.metrics.disk_percent}
                  tone="disk"
                />
                <div className="meta-line network-rate">
                  <span><Download size={14} />{t('下行网速')}</span>
                  <strong>{formatNetworkRate(host.metrics.network_rx_rate)}</strong>
                </div>
                <div className="meta-line network-rate">
                  <span><Upload size={14} />{t('上行网速')}</span>
                  <strong>{formatNetworkRate(host.metrics.network_tx_rate)}</strong>
                </div>
                <div className="meta-line">
                  <span>{t('负载')}</span>
                  <strong>{formatLoad(host.metrics.load_average)}</strong>
                </div>
                <div className="meta-line">
                  <span>{t('运行')}</span>
                  <strong>{formatDuration(host.metrics.uptime_seconds)}</strong>
                </div>
              </div>
            ) : (
              <div className="empty-inline">{t('暂无负载数据')}</div>
            )}

            <div className="public-asset-status">
              <div className="meta-line">
                <span>{t('访问延迟')}</span>
                <strong>{formatLatency(host.latency_ms)}</strong>
              </div>
              <div className="meta-line">
                <span>{t('丢包率')}</span>
                <strong>{formatPacketLoss(host.packet_loss_percent)}</strong>
              </div>
              <div className="meta-line">
                <span>{t('服务器到期')}</span>
                <strong>{formatPublicExpiry(host.expires_at, t)}</strong>
              </div>
              {(host.resolved_ipv4.length > 0 || host.resolved_ipv6.length > 0) && (
                <div className="public-addresses">
                  {host.resolved_ipv4.length > 0 && <span>IPv4 <code>{host.resolved_ipv4.join(', ')}</code></span>}
                  {host.resolved_ipv6.length > 0 && <span>IPv6 <code>{host.resolved_ipv6.join(', ')}</code></span>}
                </div>
              )}
              {host.domains.length > 0 && (
                <div className="public-domains">
                  {host.domains.map((domain) => (
                    <div className="public-domain" key={domain.id}>
                      <span><Globe2 size={13} />{domain.domain}</span>
                      <strong className={`ssl-${domain.ssl_status}`}>
                        <ShieldCheck size={13} />
                        {domain.ssl_expires_at ? formatDate(domain.ssl_expires_at) : t('待检测')}
                      </strong>
                    </div>
                  ))}
                </div>
              )}
            </div>

            {host.tags.length > 0 && (
              <div className="tag-row">
                {host.tags.map((tag) => (
                  <span className="tag" key={tag}>
                    {t(tag)}
                  </span>
                ))}
              </div>
            )}
          </article>
        ))}
      </section>
    </div>
  )
}

function formatPublicExpiry(
  iso: string | undefined,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  const days = daysUntil(iso)
  if (days === undefined) return '-'
  if (days < 0) return t('已过期')
  return t('剩余 {count} 天', { count: days })
}
