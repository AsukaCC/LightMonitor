import {
  Activity,
  CircleCheck,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Clock3,
  Cpu,
  Download,
  Edit3,
  Eye,
  ExternalLink,
  HardDrive,
  Copy,
  KeyRound,
  Link2,
  Link2Off,
  LoaderCircle,
  LogOut,
  MemoryStick,
  Network,
  Plus,
  PackageSearch,
  RefreshCw,
  RotateCcw,
  Server,
  Terminal,
  Trash2,
  TriangleAlert,
  Wifi,
  X,
} from 'lucide-react'
import type { FormEvent, ReactNode } from 'react'
import { useEffect, useMemo, useRef, useState } from 'react'
import * as d3 from 'd3'
import {
  applyRelease,
  authFetch,
  fetchHosts,
  fetchReleaseCatalog,
  fetchSession,
  login,
  logout,
  tokenStorageKey,
  userStorageKey,
} from '../api'
import { MetricBar } from '../components/MetricBar'
import { LanguageSwitcher } from '../components/LanguageSwitcher'
import { ThemeToggle } from '../components/ThemeToggle'
import { translate, useI18n } from '../i18n'
import type { Host, HostForm, MetricHistoryResponse, ReleaseCatalog, ServerEvent, ThemeMode } from '../types'
import { statusLabel } from '../types'
import {
  formatBytes,
  formatCpuDetail,
  formatDuration,
  formatLoad,
  formatRelativeTime,
  formatUsageDetail,
  isStaleHost,
  percent,
  readError,
} from '../utils'

type AuthStatus = 'checking' | 'anonymous' | 'authenticated'
type ActiveView = 'dashboard' | 'hosts' | 'versions'
type HostModalMode = 'create' | 'edit'
type HostFilter = 'all' | 'online' | 'offline' | 'never' | 'installing'
type InstallPhase = 'idle' | 'installing' | 'success' | 'error'
type InstallAuth = 'saved' | 'identity' | 'password' | 'key'
type HistoryRange = '1h' | '4h' | '6h' | '12h' | '1d'
type HistoryView = 'resources' | 'network' | 'load'
type DeleteFallback = { ids: string[]; message: string }

const initialForm: HostForm = {
  name: '',
  address: '',
  ssh_user: '',
  ssh_port: '22',
  ssh_password: '',
  clear_ssh_password: false,
  tags: '',
}

const hostPageSize = 10
const intervalPresets = [5, 10, 30, 60, 300]
const manualUninstallCommand = `sudo systemctl disable --now lightmonitor-agent || true
sudo rm -f /etc/systemd/system/lightmonitor-agent.service
sudo systemctl daemon-reload
sudo rm -rf /opt/lightmonitor`

export function AdminPage({
  theme,
  onToggleTheme,
}: {
  theme: ThemeMode
  onToggleTheme: () => void
}) {
  const { language, t } = useI18n()
  const [hosts, setHosts] = useState<Host[]>([])
  const [detailHostId, setDetailHostId] = useState<string>()
  const [detailTab, setDetailTab] = useState<'info' | 'load' | 'history' | 'logs'>('info')
  const [selectedHostIds, setSelectedHostIds] = useState<string[]>([])
  const [intervalHostIds, setIntervalHostIds] = useState<string[]>([])
  const [intervalSeconds, setIntervalSeconds] = useState('5')
  const [hostModalMode, setHostModalMode] = useState<HostModalMode>()
  const [editingHostId, setEditingHostId] = useState<string>()
  const [installHostId, setInstallHostId] = useState<string>()
  const [tokenInfo, setTokenInfo] = useState<{ agent_token: string; install_command: string }>()
  const [tokenLoading, setTokenLoading] = useState(false)
  const [manualInstallOpen, setManualInstallOpen] = useState(false)
  const [deleteFallback, setDeleteFallback] = useState<DeleteFallback>()
  const [installPhase, setInstallPhase] = useState<InstallPhase>('idle')
  const [installError, setInstallError] = useState('')
  const [manualInstallError, setManualInstallError] = useState('')
  const [copiedItem, setCopiedItem] = useState<string>()
  const [hostPage, setHostPage] = useState(1)
  const [form, setForm] = useState<HostForm>(initialForm)
  const [installKeyPath, setInstallKeyPath] = useState('')
  const [installPassword, setInstallPassword] = useState('')
  const [installAuth, setInstallAuth] = useState<InstallAuth>('password')
  const [loginForm, setLoginForm] = useState({ username: '', password: '' })
  const [token, setToken] = useState(() => sessionStorage.getItem(tokenStorageKey) ?? '')
  const [adminUser, setAdminUser] = useState(() => sessionStorage.getItem(userStorageKey) ?? '')
  const [authStatus, setAuthStatus] = useState<AuthStatus>(() =>
    sessionStorage.getItem(tokenStorageKey) ? 'checking' : 'anonymous',
  )
  const [activeView, setActiveView] = useState<ActiveView>('dashboard')
  const [hostFilter, setHostFilter] = useState<HostFilter>('all')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [loginError, setLoginError] = useState('')
  const [tick, setTick] = useState(0)

  const detailHost = useMemo(
    () => hosts.find((host) => host.id === detailHostId),
    [hosts, detailHostId],
  )
  const installHost = useMemo(
    () => hosts.find((host) => host.id === installHostId),
    [hosts, installHostId],
  )
  const intervalHosts = useMemo(
    () => hosts.filter((host) => intervalHostIds.includes(host.id)),
    [hosts, intervalHostIds],
  )

  const summary = useMemo(
    () => ({
      total: hosts.length,
      online: hosts.filter((host) => host.status === 'online').length,
      installing: hosts.filter((host) => host.status === 'installing').length,
      error: hosts.filter((host) => host.status === 'error' || host.status === 'offline').length,
      never: hosts.filter((host) => !host.agent_id).length,
    }),
    [hosts],
  )

  const filteredHosts = useMemo(() => {
    switch (hostFilter) {
      case 'online':
        return hosts.filter((h) => h.status === 'online')
      case 'offline':
        return hosts.filter((h) => h.status === 'offline' || h.status === 'error')
      case 'never':
        return hosts.filter((h) => !h.agent_id)
      case 'installing':
        return hosts.filter((h) => h.status === 'installing')
      default:
        return hosts
    }
  }, [hosts, hostFilter])

  const totalHostPages = Math.max(1, Math.ceil(filteredHosts.length / hostPageSize))
  const hostPageStart = (hostPage - 1) * hostPageSize
  const pagedHosts = filteredHosts.slice(hostPageStart, hostPageStart + hostPageSize)
  const selectablePagedHosts = pagedHosts.filter((host) => !host.is_system)
  const allPageHostsSelected =
    selectablePagedHosts.length > 0 && selectablePagedHosts.every((host) => selectedHostIds.includes(host.id))
  // keep relative times fresh
  void tick

  useEffect(() => {
    setHostPage((current) => Math.min(current, totalHostPages))
  }, [totalHostPages])

  useEffect(() => {
    if (detailHostId) {
      setDetailTab('info')
    }
  }, [detailHostId])

  useEffect(() => {
    setHostPage(1)
  }, [hostFilter])

  useEffect(() => {
    const timer = window.setInterval(() => setTick((n) => n + 1), 15000)
    return () => window.clearInterval(timer)
  }, [])

  useEffect(() => {
    let eventController: AbortController | undefined
    let cancelled = false

    if (!token) {
      setAuthStatus('anonymous')
      return undefined
    }

    async function verifyAndConnect() {
      setAuthStatus('checking')
      setLoginError('')
      try {
        const session = await fetchSession(token)
        if (cancelled) return
        setAdminUser(session.username)
        sessionStorage.setItem(userStorageKey, session.username)
        const data = await fetchHosts(token, clearSession)
        if (cancelled) return
        setHosts(data)
        setAuthStatus('authenticated')

        eventController = new AbortController()
        void consumeServerEvents(token, eventController.signal, clearSession, (payload) => {
          if (payload.type === 'host_updated') upsertHost(payload.host)
          if (payload.type === 'hosts_deleted') removeHosts(payload.host_ids)
          if (payload.type === 'install_log') {
            setHosts((current) =>
              current.map((host) =>
                host.id === payload.host_id
                  ? { ...host, install_logs: [...host.install_logs, payload.log] }
                  : host,
              ),
            )
          }
        })
      } catch (err) {
        if (!cancelled) {
          setLoginError(err instanceof Error ? err.message : t('认证失败'))
          clearSession()
        }
      }
    }

    void verifyAndConnect()
    return () => {
      cancelled = true
      eventController?.abort()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [token])

  function clearSession() {
    sessionStorage.removeItem(tokenStorageKey)
    sessionStorage.removeItem(userStorageKey)
    setToken('')
    setAdminUser('')
    setAuthStatus('anonymous')
    setHosts([])
    setDetailHostId(undefined)
    setSelectedHostIds([])
    setIntervalHostIds([])
    setDeleteFallback(undefined)
    closeHostModal()
    closeInstallModal()
  }

  async function loadHosts() {
    const data = await fetchHosts(token, clearSession)
    setHosts(data)
    setSelectedHostIds((current) =>
      current.filter((id) => data.some((host) => host.id === id && !host.is_system)),
    )
  }

  async function submitLogin(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setBusy(true)
    setLoginError('')
    try {
      const data = await login(loginForm.username, loginForm.password)
      sessionStorage.setItem(tokenStorageKey, data.token)
      sessionStorage.setItem(userStorageKey, data.username)
      setToken(data.token)
      setAdminUser(data.username)
      setAuthStatus('checking')
      setLoginForm({ username: '', password: '' })
    } catch (err) {
      setLoginError(err instanceof Error ? err.message : t('登录失败'))
    } finally {
      setBusy(false)
    }
  }

  async function handleLogout() {
    if (token) await logout(token)
    clearSession()
  }

  function upsertHost(next: Host) {
    setHosts((current) => {
      const existing = current.find((host) => host.id === next.id)
      const nextHost =
        existing && next.name.trim().length === 0 && existing.name.trim().length > 0
          ? { ...next, name: existing.name }
          : next
      if (existing && existing.latest && nextHost.latest) {
        const prev = existing.latest
        const curr = nextHost.latest
        const elapsed = (new Date(curr.collected_at).getTime() - new Date(prev.collected_at).getTime()) / 1000
        if (elapsed > 0.5) {
          const rxDelta = curr.network_rx_bytes >= prev.network_rx_bytes ? curr.network_rx_bytes - prev.network_rx_bytes : 0
          const txDelta = curr.network_tx_bytes >= prev.network_tx_bytes ? curr.network_tx_bytes - prev.network_tx_bytes : 0
          curr.network_rx_rate = rxDelta / elapsed
          curr.network_tx_rate = txDelta / elapsed
        } else {
          curr.network_rx_rate = prev.network_rx_rate
          curr.network_tx_rate = prev.network_tx_rate
        }
      }
      const exists = current.some((host) => host.id === nextHost.id)
      if (!exists) return [nextHost, ...current]
      return current.map((host) => (host.id === nextHost.id ? nextHost : host))
    })
  }

  function removeHosts(ids: string[]) {
    const deleted = new Set(ids)
    setHosts((current) => {
      return current.filter((host) => !deleted.has(host.id))
    })
    setDetailHostId((current) => (current && deleted.has(current) ? undefined : current))
    setSelectedHostIds((current) => current.filter((id) => !deleted.has(id)))
  }

  function openCreateHostModal() {
    setForm(initialForm)
    setEditingHostId(undefined)
    setError('')
    setHostModalMode('create')
  }

  function openEditHostModal(host: Host) {
    setForm({
      name: host.name,
      address: host.address,
      ssh_user: host.ssh_user,
      ssh_port: String(host.ssh_port),
      ssh_password: '',
      clear_ssh_password: false,
      tags: host.tags.join(', '),
    })
    setEditingHostId(host.id)
    setError('')
    setHostModalMode('edit')
  }

  function openIntervalModal(ids: string[]) {
    const targets = hosts.filter((host) => ids.includes(host.id))
    if (targets.length === 0) return
    const firstInterval = targets[0].update_interval_seconds
    const sharedInterval = targets.every((host) => host.update_interval_seconds === firstInterval)
    setIntervalHostIds(targets.map((host) => host.id))
    setIntervalSeconds(String(sharedInterval ? firstInterval : 5))
    setError('')
  }

  function closeIntervalModal() {
    if (busy) return
    setIntervalHostIds([])
    setIntervalSeconds('5')
    setError('')
  }

  function closeHostModal() {
    setHostModalMode(undefined)
    setEditingHostId(undefined)
    setForm(initialForm)
  }

  function openInstallModal(host: Host) {
    if (host.is_system) return
    setInstallHostId(host.id)
    setInstallAuth(host.has_ssh_identity ? 'identity' : host.has_ssh_password ? 'saved' : 'password')
    setInstallPassword('')
    setInstallKeyPath('/root/.ssh/id_rsa')
    setInstallPhase(host.status === 'installing' ? 'installing' : 'idle')
    setInstallError('')
    setManualInstallOpen(false)
    setTokenInfo(undefined)
    setTokenLoading(false)
    setManualInstallError('')
    setError('')
  }

  function closeInstallModal() {
    if (installPhase === 'installing') return
    setInstallHostId(undefined)
    setInstallPassword('')
    setTokenInfo(undefined)
    setManualInstallOpen(false)
    setInstallError('')
    setManualInstallError('')
    setInstallPhase('idle')
  }

  async function loadManualInstall() {
    if (!installHostId || tokenInfo || tokenLoading) return
    setTokenLoading(true)
    setManualInstallError('')
    try {
      const response = await authFetch(`/api/hosts/${installHostId}/agent-token`, undefined, token, clearSession)
      if (!response.ok) throw new Error(await readError(response))
      const data = (await response.json()) as { agent_token: string; install_command: string }
      setTokenInfo(data)
    } catch (err) {
      setManualInstallError(err instanceof Error ? err.message : t('获取手动安装命令失败'))
    } finally {
      setTokenLoading(false)
    }
  }

  function toggleManualInstall() {
    const next = !manualInstallOpen
    setManualInstallOpen(next)
    if (next) void loadManualInstall()
  }

  async function copyText(text: string, item: string) {
    try {
      await navigator.clipboard.writeText(text)
      setCopiedItem(item)
      window.setTimeout(() => {
        setCopiedItem((current) => (current === item ? undefined : current))
      }, 1800)
    } catch {
      setManualInstallError(t('复制失败，请手动选择文本'))
    }
  }

  async function submitHost(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setBusy(true)
    setError('')
    try {
      const response = await authFetch(
        hostModalMode === 'edit' ? `/api/hosts/${editingHostId}` : '/api/hosts',
        {
          method: hostModalMode === 'edit' ? 'PUT' : 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            name: form.name,
            address: form.address,
            // Region is resolved server-side from the current address.
            region: '',
            ssh_user: form.ssh_user.trim(),
            ssh_port: Number(form.ssh_port || 22),
            ssh_password: form.ssh_password,
            clear_ssh_password: form.clear_ssh_password,
            tags: form.tags
              .split(',')
              .map((tag) => tag.trim())
              .filter(Boolean),
          }),
        },
        token,
        clearSession,
      )
      if (!response.ok) throw new Error(await readError(response))
      const saved = (await response.json()) as Host
      upsertHost(saved)
      closeHostModal()
    } catch (err) {
      setError(err instanceof Error ? err.message : t('请求失败'))
    } finally {
      setBusy(false)
    }
  }

  async function submitInterval(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const seconds = Number(intervalSeconds)
    if (!Number.isInteger(seconds) || seconds < 1 || seconds > 3600) {
      setError(t('更新间隔必须是 1–3600 秒的整数'))
      return
    }

    setBusy(true)
    setError('')
    try {
      const response = await authFetch(
        '/api/hosts/update-interval',
        {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ ids: intervalHostIds, interval_seconds: seconds }),
        },
        token,
        clearSession,
      )
      if (!response.ok) throw new Error(await readError(response))
      const updated = (await response.json()) as Host[]
      const updatedById = new Map(updated.map((host) => [host.id, host]))
      setHosts((current) => current.map((host) => updatedById.get(host.id) ?? host))
      setIntervalHostIds([])
      setIntervalSeconds('5')
    } catch (err) {
      setError(err instanceof Error ? err.message : t('更新频率设置失败'))
    } finally {
      setBusy(false)
    }
  }

  async function submitInstall(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!installHostId) return
    setInstallPhase('installing')
    setInstallError('')
    try {
      if (installAuth === 'password' && !installPassword.trim()) {
        throw new Error(t('请填写 SSH 密码'))
      }
      if (installAuth === 'key' && !installKeyPath.trim()) {
        throw new Error(t('请填写 SSH 私钥路径'))
      }
      const response = await authFetch(
        `/api/hosts/${installHostId}/install-agent`,
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            ssh_key_path: installAuth === 'key' ? installKeyPath : '',
            ssh_password: installAuth === 'password' ? installPassword : '',
            use_saved_identity: installAuth === 'identity',
          }),
        },
        token,
        clearSession,
      )
      if (!response.ok) throw new Error(await readError(response))
      const host = (await response.json()) as Host
      upsertHost(host)
      setInstallPhase('success')
    } catch (err) {
      setInstallPhase('error')
      setInstallError(err instanceof Error ? err.message : t('安装失败，请检查 SSH 配置后重试'))
    }
  }

  async function deleteHostsByIds(ids: string[], confirmMsg?: string, force = false) {
    const deletableIds = ids.filter((id) => !hosts.some((host) => host.id === id && host.is_system))
    if (deletableIds.length === 0) return
    if (!force && !window.confirm(confirmMsg ?? t('确认删除 {count} 台主机？将先卸载远端探针，再删除记录。', { count: deletableIds.length }))) {
      return
    }
    setBusy(true)
    setError('')
    try {
      const response = await authFetch(
        '/api/hosts',
        {
          method: 'DELETE',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ ids: deletableIds, force }),
        },
        token,
        clearSession,
      )
      if (!response.ok) throw new Error(await readError(response))
      removeHosts(deletableIds)
      setDeleteFallback(undefined)
    } catch (err) {
      await loadHosts().catch(() => undefined)
      const message = err instanceof Error ? err.message : t('删除失败')
      setError(message)
      if (!force) setDeleteFallback({ ids: deletableIds, message })
    } finally {
      setBusy(false)
    }
  }

  async function deleteSelectedHosts() {
    await deleteHostsByIds(selectedHostIds)
  }

  async function deleteOneHost(host: Host) {
    if (host.is_system) return
    await deleteHostsByIds(
      [host.id],
      t('确认删除主机「{name}」？\n地址 {address}\n将先卸载远端探针，再删除记录。', { name: host.name, address: host.address }),
    )
  }

  async function forceDeleteFallback() {
    if (!deleteFallback) return
    await deleteHostsByIds(deleteFallback.ids, undefined, true)
  }

  function navigate(view: ActiveView) {
    setActiveView(view)
  }

  if (authStatus !== 'authenticated') {
    return (
      <main className="login-shell">
        <form className="login-card" onSubmit={submitLogin}>
          <div className="brand">
            <div className="brand-mark">
              <Activity size={18} />
            </div>
            <div>
              <h1>LightMonitor</h1>
              <span>{t('管理后台')}</span>
            </div>
          </div>
          <label>
            {t('账户')}
            <input
              autoComplete="username"
              required
              value={loginForm.username}
              onChange={(e) => setLoginForm({ ...loginForm, username: e.target.value })}
            />
          </label>
          <label>
            {t('密码')}
            <div className="input-with-icon">
              <KeyRound size={15} />
              <input
                autoComplete="current-password"
                required
                type="password"
                value={loginForm.password}
                onChange={(e) => setLoginForm({ ...loginForm, password: e.target.value })}
              />
            </div>
          </label>
          {loginError && <div className="banner error">{loginError}</div>}
          <button className="btn primary" disabled={busy || authStatus === 'checking'} type="submit">
            {busy || authStatus === 'checking' ? t('验证中…') : t('登录')}
          </button>
          <div className="login-footer">
            <a href="/">{t('返回公开监控')}</a>
            <div className="login-footer-actions">
              <LanguageSwitcher />
              <ThemeToggle theme={theme} onToggle={onToggleTheme} />
            </div>
          </div>
        </form>
      </main>
    )
  }

  return (
    <div className="admin-shell">
      <header className="top-nav">
        <div className="nav-container">
          <div className="brand">
            <div className="brand-mark">
              <Activity size={18} />
            </div>
            <div>
              <h1>LightMonitor</h1>
            </div>
          </div>

          <nav className="nav-links">
            <button
              className={activeView === 'dashboard' ? 'active' : ''}
              onClick={() => navigate('dashboard')}
              type="button"
            >
              <Activity size={16} />
              {t('仪表盘')}
            </button>
            <button
              className={activeView === 'hosts' ? 'active' : ''}
              onClick={() => navigate('hosts')}
              type="button"
            >
              <Server size={16} />
              {t('主机管理')}
            </button>
            <button
              className={activeView === 'versions' ? 'active' : ''}
              onClick={() => navigate('versions')}
              type="button"
            >
              <PackageSearch size={16} />
              {t('版本管理')}
            </button>
          </nav>

          <div className="nav-actions">
            <span className="muted small hide-sm">{adminUser}</span>
            <LanguageSwitcher />
            <ThemeToggle theme={theme} onToggle={onToggleTheme} />
            <button className="icon-btn" onClick={() => void loadHosts()} title={t('刷新')} type="button">
              <RefreshCw size={16} />
            </button>
            <button className="icon-btn" onClick={() => void handleLogout()} title={t('退出')} type="button">
              <LogOut size={16} />
            </button>
          </div>
        </div>
      </header>

      <main className="admin-main">
        <header className="page-header">
          <h2>{activeView === 'dashboard' ? t('主机概览') : activeView === 'hosts' ? t('主机列表') : t('版本管理')}</h2>
          <div className="page-actions">
            <a className="btn ghost" href="/">
              {t('公开监控')}
            </a>
          </div>
        </header>

        {activeView === 'dashboard' ? (
          <>
            <section className="summary-row">
              <SummaryCard
                icon={<Server size={18} />}
                label={t('主机总数')}
                value={summary.total}
              />
              <SummaryCard
                icon={<Wifi size={18} />}
                label={t('在线')}
                value={summary.online}
                tone="online"
              />
              <SummaryCard
                icon={<Terminal size={18} />}
                label={t('安装中')}
                value={summary.installing}
                tone="installing"
              />
              <SummaryCard
                icon={<Activity size={18} />}
                label={t('离线/异常')}
                value={summary.error}
                tone="offline"
              />
            </section>

            <section className="dashboard-host-section">
              <div className="section-head" style={{ marginBottom: '14px' }}>
                <h3>{t('主机列表')}</h3>
                <span className="muted small">
                  {t('显示 {shown} / {total} 台', { shown: filteredHosts.length, total: hosts.length })}
                </span>
              </div>
              <div className="filter-tabs">
                {(
                  [
                    ['all', `${t('全部')} ${summary.total}`],
                    ['online', `${t('在线')} ${summary.online}`],
                    ['installing', `${t('安装中')} ${summary.installing}`],
                    ['offline', `${t('离线/异常')} ${summary.error}`],
                    ['never', `${t('未连接')} ${summary.never}`],
                  ] as const
                ).map(([key, label]) => (
                  <button
                    className={hostFilter === key ? 'active' : ''}
                    key={key}
                    onClick={() => setHostFilter(key)}
                    type="button"
                  >
                    {label}
                  </button>
                ))}
              </div>
              {hosts.length === 0 ? (
                <div className="empty-inline">{t('暂无主机，请先创建')}</div>
              ) : filteredHosts.length === 0 ? (
                <div className="empty-inline">{t('暂无匹配主机')}</div>
              ) : (
                <div className="dashboard-host-list">
                  {filteredHosts.map((host) => (
                    <article className="dashboard-host" key={host.id}>
                      <div className="dashboard-host-head">
                        <div className="dashboard-host-identity">
                          <span className={`dot ${host.status}`} />
                          <div className="dashboard-host-copy">
                            <h3 className="host-name">
                              <span className="host-name-text">{host.name}</span>
                              {host.is_system && <span className="tag">{t('宿主机')}</span>}
                            </h3>
                            <p className="dashboard-host-meta">
                              {host.address} · {host.region || t('未设置地区')} · {formatRelativeTime(host.last_seen)}
                            </p>
                          </div>
                        </div>
                        <div className="dashboard-host-actions">
                          <span className={`status-pill ${host.status}`}>{statusLabel(host.status)}</span>
                          <button className="btn secondary" onClick={() => setDetailHostId(host.id)} type="button">
                            <Eye size={15} />
                            {t('详情')}
                          </button>
                          <button
                            className="icon-btn"
                            disabled={host.is_system || host.status === 'installing'}
                            onClick={() => openInstallModal(host)}
                            title={host.is_system ? t('使用内置采集') : host.status === 'installing' ? t('探针安装中') : t('安装探针')}
                            type="button"
                          >
                            {host.status === 'installing' ? <LoaderCircle className="spin" size={15} /> : <Download size={15} />}
                          </button>
                        </div>
                      </div>
                      <HostMetricSummary host={host} />
                    </article>
                  ))}
                </div>
              )}
            </section>
          </>
        ) : activeView === 'hosts' ? (
          <section className="panel">
            <div className="panel-head wrap">
              <h3>{t('主机列表')}</h3>
              <div className="panel-actions">
                <button
                  className="btn secondary"
                  disabled={selectedHostIds.length === 0 || busy}
                  onClick={() => openIntervalModal(selectedHostIds)}
                  type="button"
                >
                  <Clock3 size={15} />
                  {t('设置数据更新')} ({selectedHostIds.length})
                </button>
                <button
                  className="btn secondary danger"
                  disabled={selectedHostIds.length === 0 || busy}
                  onClick={() => void deleteSelectedHosts()}
                  type="button"
                >
                  <Trash2 size={15} />
                  {t('删除选中 ({count})', { count: selectedHostIds.length })}
                </button>
                <button className="btn primary" onClick={openCreateHostModal} type="button">
                  <Plus size={15} />
                  {t('新建')}
                </button>
              </div>
            </div>

            <div className="filter-tabs">
              {(
                [
                  ['all', `${t('全部')} ${summary.total}`],
                  ['online', `${t('在线')} ${summary.online}`],
                  ['installing', `${t('安装中')} ${summary.installing}`],
                  ['offline', `${t('离线')} ${summary.error}`],
                  ['never', `${t('未连接')} ${summary.never}`],
                ] as const
              ).map(([key, label]) => (
                <button
                  className={hostFilter === key ? 'active' : ''}
                  key={key}
                  onClick={() => setHostFilter(key)}
                  type="button"
                >
                  {label}
                </button>
              ))}
            </div>

            <div className="table-wrap">
              <table className="data-table">
                <thead>
                  <tr>
                    <th>
                      <input
                        checked={allPageHostsSelected}
                        disabled={selectablePagedHosts.length === 0}
                        onChange={() => {
                          const pageIds = selectablePagedHosts.map((host) => host.id)
                          setSelectedHostIds((current) =>
                            allPageHostsSelected
                              ? current.filter((id) => !pageIds.includes(id))
                              : Array.from(new Set([...current, ...pageIds])),
                          )
                        }}
                        type="checkbox"
                      />
                    </th>
                    <th>{t('名称')}</th>
                    <th>{t('地区')}</th>
                    <th>{t('地址')}</th>
                    <th>{t('探针')}</th>
                    <th>{t('最后上报')}</th>
                    <th>{t('数据更新')}</th>
                    <th>{t('状态')}</th>
                    <th>{t('标签')}</th>
                    <th>{t('操作')}</th>
                  </tr>
                </thead>
                <tbody>
                  {pagedHosts.map((host) => {
                    const connected = Boolean(host.agent_id)
                    return (
                      <tr className={isStaleHost(host.status, connected) ? 'stale-row' : ''} key={host.id}>
                        <td data-label={t('选择')}>
                          <input
                            aria-label={t('选择主机 {name}', { name: host.name })}
                            checked={selectedHostIds.includes(host.id)}
                            disabled={host.is_system}
                            onChange={() =>
                              setSelectedHostIds((current) =>
                                current.includes(host.id)
                                  ? current.filter((id) => id !== host.id)
                                  : [...current, host.id],
                              )
                            }
                            title={host.is_system ? t('宿主机不可删除') : undefined}
                            type="checkbox"
                          />
                        </td>
                        <td data-label={t('名称')}>
                          <div className="host-name">
                            <button
                              className="link-btn host-name-text"
                              onClick={() => {
                                setDetailHostId(host.id)
                                setActiveView('dashboard')
                              }}
                              type="button"
                            >
                              {host.name}
                            </button>
                            {host.is_system && <span className="tag">{t('宿主机')}</span>}
                          </div>
                        </td>
                        <td data-label={t('地区')}>{host.region || '-'}</td>
                        <td className="mono" data-label={t('地址')}>{host.address}</td>
                        <td data-label={t('探针')}>
                          <span className={`conn-pill ${connected ? 'on' : 'off'}`}>
                            {connected ? <Link2 size={13} /> : <Link2Off size={13} />}
                            {host.is_system ? t('内置采集') : connected ? t('已注册') : t('未连接')}
                          </span>
                        </td>
                        <td data-label={t('最后上报')} title={host.last_seen ? new Date(host.last_seen).toLocaleString(language) : ''}>
                          {formatRelativeTime(host.last_seen)}
                        </td>
                        <td data-label={t('数据更新')}>{formatUpdateInterval(host.update_interval_seconds)}</td>
                        <td data-label={t('状态')}>
                          <span className={`status-pill ${host.status}`}>{statusLabel(host.status)}</span>
                        </td>
                        <td data-label={t('标签')}>{host.tags.length ? host.tags.map((tag) => t(tag)).join(', ') : '-'}</td>
                        <td data-label={t('操作')}>
                          <div className="row-actions">
                            <button
                              className="icon-btn"
                              onClick={() => openEditHostModal(host)}
                              title={t('编辑')}
                              type="button"
                            >
                              <Edit3 size={15} />
                            </button>
                            <button
                              className="icon-btn"
                              onClick={() => openIntervalModal([host.id])}
                              title={t('设置数据更新')}
                              type="button"
                            >
                              <Clock3 size={15} />
                            </button>
                            <button
                              className="icon-btn"
                              disabled={host.is_system || host.status === 'installing'}
                              onClick={() => openInstallModal(host)}
                              title={host.is_system ? t('使用内置采集') : host.status === 'installing' ? t('探针安装中') : t('安装探针')}
                              type="button"
                            >
                              {host.status === 'installing' ? <LoaderCircle className="spin" size={15} /> : <Download size={15} />}
                            </button>
                            <button
                              className="icon-btn danger"
                              disabled={host.is_system || busy}
                              onClick={() => void deleteOneHost(host)}
                              title={host.is_system ? t('宿主机不可删除') : t('删除')}
                              type="button"
                            >
                              <Trash2 size={15} />
                            </button>
                          </div>
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
              {hosts.length === 0 && <div className="empty-inline">{t('暂无主机')}</div>}
              {hosts.length > 0 && filteredHosts.length === 0 && (
                <div className="empty-inline">{t('当前筛选下无主机')}</div>
              )}
            </div>

            <div className="pagination">
              <span className="muted small">
                {t('第 {page}/{pages} 页 · 筛选 {filtered} / 共 {total} 台', {
                  page: hostPage, pages: totalHostPages, filtered: filteredHosts.length, total: hosts.length,
                })}
              </span>
              <div className="row-actions">
                <button
                  className="icon-btn"
                  disabled={hostPage <= 1}
                  onClick={() => setHostPage((c) => Math.max(1, c - 1))}
                  type="button"
                >
                  <ChevronLeft size={18} />
                </button>
                <button
                  className="icon-btn"
                  disabled={hostPage >= totalHostPages}
                  onClick={() => setHostPage((c) => Math.min(totalHostPages, c + 1))}
                  type="button"
                >
                  <ChevronRight size={18} />
                </button>
              </div>
            </div>
            {error && <div className="banner error">{error}</div>}
          </section>
        ) : (
          <VersionPanel onUnauthorized={clearSession} token={token} />
        )}
      </main>

      {deleteFallback && (
        <div className="modal-backdrop">
          <div className="modal">
            <div className="modal-head">
              <div>
                <h3>{t('手动卸载探针')}</h3>
                <span className="muted small">{t('{count} 台主机待处理', { count: deleteFallback.ids.length })}</span>
              </div>
              <button
                className="icon-btn"
                disabled={busy}
                onClick={() => setDeleteFallback(undefined)}
                title={t('关闭')}
                type="button"
              >
                <X size={18} />
              </button>
            </div>
            <div className="modal-body">
              <div className="banner error">{deleteFallback.message}</div>
              <label>
                <span>{t('手动卸载命令')}</span>
                <span className="copy-help">{t('请在待删除探针所在的远程主机终端执行。')}</span>
                <div className="copy-row">
                  <textarea className="mono cmd-box" readOnly rows={5} value={manualUninstallCommand} />
                  <button
                    aria-label={t('复制手动卸载命令')}
                    className="icon-btn"
                    onClick={() => void copyText(manualUninstallCommand, 'uninstall-command')}
                    title={t('复制卸载命令')}
                    type="button"
                  >
                    {copiedItem === 'uninstall-command' ? <CircleCheck size={15} /> : <Copy size={15} />}
                  </button>
                </div>
                {copiedItem === 'uninstall-command' && <span className="copy-feedback">{t('已复制：手动卸载命令')}</span>}
              </label>
            </div>
            <div className="modal-actions">
              <button
                className="btn secondary"
                disabled={busy}
                onClick={() => setDeleteFallback(undefined)}
                type="button"
              >
                {t('取消')}
              </button>
              <button
                className="btn secondary danger"
                disabled={busy}
                onClick={() => void forceDeleteFallback()}
                type="button"
              >
                {busy && <LoaderCircle className="spin" size={15} />}
                {t('已卸载，删除记录')}
              </button>
            </div>
          </div>
        </div>
      )}

      {detailHost && (
        <div className="modal-backdrop">
          <div className="modal detail-modal">
            <div className="modal-head">
              <div>
                <h3>{detailHost.name}</h3>
                <span className="muted small">{t('主机详情')}</span>
              </div>
              <button className="icon-btn" onClick={() => setDetailHostId(undefined)} title={t('关闭')} type="button">
                <X size={18} />
              </button>
            </div>
            <div className="filter-tabs" style={{ marginBottom: 0 }}>
              <button
                className={detailTab === 'info' ? 'active' : ''}
                onClick={() => setDetailTab('info')}
                type="button"
              >
                {t('主机信息')}
              </button>
              <button
                className={detailTab === 'load' ? 'active' : ''}
                onClick={() => setDetailTab('load')}
                type="button"
              >
                {t('主机负载')}
              </button>
              <button
                className={detailTab === 'history' ? 'active' : ''}
                onClick={() => setDetailTab('history')}
                type="button"
              >
                {t('历史趋势')}
              </button>
              <button
                className={detailTab === 'logs' ? 'active' : ''}
                onClick={() => setDetailTab('logs')}
                type="button"
              >
                {t('日志信息')}
              </button>
            </div>
            <div className="modal-body">
              {detailTab === 'history' ? (
                <HostHistoryPanel
                  host={detailHost}
                  key={detailHost.id}
                  onUnauthorized={clearSession}
                  token={token}
                />
              ) : (
                <HostDetailContent host={detailHost} tab={detailTab} />
              )}
            </div>
            <div className="modal-actions">
              <button className="btn secondary" onClick={() => setDetailHostId(undefined)} type="button">
                {t('关闭')}
              </button>
            </div>
          </div>
        </div>
      )}

      {intervalHostIds.length > 0 && (
        <div className="modal-backdrop">
          <form className="modal interval-modal" onSubmit={submitInterval}>
            <div className="modal-head">
              <div>
                <h3>{intervalHosts.length === 1 ? t('设置数据更新') : t('批量设置数据更新')}</h3>
                <span className="muted small">
                  {intervalHosts.length === 1 ? intervalHosts[0]?.name : t('{count} 台主机', { count: intervalHosts.length })}
                </span>
              </div>
              <button className="icon-btn" disabled={busy} onClick={closeIntervalModal} title={t('关闭')} type="button">
                <X size={18} />
              </button>
            </div>
            <div className="modal-body">
              <div className="filter-tabs interval-presets">
                {intervalPresets.map((seconds) => (
                  <button
                    className={Number(intervalSeconds) === seconds ? 'active' : ''}
                    disabled={busy}
                    key={seconds}
                    onClick={() => setIntervalSeconds(String(seconds))}
                    type="button"
                  >
                    {formatUpdateInterval(seconds)}
                  </button>
                ))}
              </div>
              <label>
                {t('更新间隔（秒）')}
                <input
                  disabled={busy}
                  inputMode="numeric"
                  max="3600"
                  min="1"
                  required
                  type="number"
                  value={intervalSeconds}
                  onChange={(event) => setIntervalSeconds(event.target.value)}
                />
              </label>
              {error && <div className="banner error">{error}</div>}
            </div>
            <div className="modal-actions">
              <button className="btn secondary" disabled={busy} onClick={closeIntervalModal} type="button">
                {t('取消')}
              </button>
              <button className="btn primary" disabled={busy} type="submit">
                {busy ? <LoaderCircle className="spin" size={15} /> : <Clock3 size={15} />}
                {busy ? t('保存中') : t('保存设置')}
              </button>
            </div>
          </form>
        </div>
      )}

      {hostModalMode && (
        <div className="modal-backdrop">
          <form className="modal" onSubmit={submitHost}>
            <div className="modal-head">
              <h3>{hostModalMode === 'edit' ? t('编辑主机') : t('新建主机')}</h3>
              <button className="icon-btn" onClick={closeHostModal} type="button">
                <X size={18} />
              </button>
            </div>
            <div className="modal-body">
              <div className="form-grid">
                <label>
                  {t('名称')}
                    <input required value={form.name} onChange={(e) => setForm((current) => ({ ...current, name: e.target.value }))} />
                </label>
                <label>
                  {t('IP / 域名')}
                  <input
                    required
                    placeholder={t('IP 或域名')}
                    value={form.address}
                    onChange={(e) => setForm((current) => ({ ...current, address: e.target.value }))}
                  />
                </label>
                <label>
                  {t('SSH 端口')}
                  <input
                    min="1"
                    max="65535"
                    inputMode="numeric"
                    required
                    type="number"
                    value={form.ssh_port}
                    onChange={(e) => setForm((current) => ({ ...current, ssh_port: e.target.value }))}
                  />
                </label>
                <label>
                  {t('SSH 账户（可选）')}
                  <input
                    autoComplete="username"
                    placeholder={t('留空')}
                    value={form.ssh_user}
                    onChange={(e) => setForm((current) => ({ ...current, ssh_user: e.target.value }))}
                  />
                </label>
                <label className="wide">
                  {t('SSH 密码（可选）')}
                  <input
                    autoComplete="new-password"
                    type="password"
                    placeholder={
                      hostModalMode === 'edit'
                        ? t('留空保持不变；填写则更新')
                        : t('用于远程安装探针（可选，可稍后填写）')
                    }
                    value={form.ssh_password}
                    onChange={(e) =>
                      setForm((current) => ({ ...current, ssh_password: e.target.value, clear_ssh_password: false }))
                    }
                  />
                </label>
                {hostModalMode === 'edit' && (
                  <label className="wide checkbox-row">
                    <input
                      checked={form.clear_ssh_password}
                      onChange={(e) =>
                        setForm((current) => ({
                          ...current,
                          clear_ssh_password: e.target.checked,
                          ssh_password: e.target.checked ? '' : current.ssh_password,
                        }))
                      }
                      type="checkbox"
                    />
                    {t('清除已保存的 SSH 密码')}
                    {hosts.find((h) => h.id === editingHostId)?.has_ssh_password ? t('（当前已保存）') : t('（当前未保存）')}
                  </label>
                )}
                <label className="wide">
                  {t('标签（逗号分隔）')}
                  <input value={form.tags} onChange={(e) => setForm((current) => ({ ...current, tags: e.target.value }))} />
                </label>
              </div>
              {error && <div className="banner error">{error}</div>}
            </div>
            <div className="modal-actions">
              <button className="btn secondary" onClick={closeHostModal} type="button">
                {t('取消')}
              </button>
              <button className="btn primary" disabled={busy} type="submit">
                {busy ? t('保存中…') : t('保存')}
              </button>
            </div>
          </form>
        </div>
      )}

      {installHostId && (
        <div className="modal-backdrop">
          <form className="modal install-modal" onSubmit={submitInstall}>
            <div className="modal-head">
              <div>
                <h3>{t('安装探针')}</h3>
                <span className="muted small">{installHost?.name}</span>
              </div>
              <button
                className="icon-btn"
                disabled={installPhase === 'installing'}
                onClick={closeInstallModal}
                title={t('关闭')}
                type="button"
              >
                <X size={18} />
              </button>
            </div>
            <div className="modal-body">
              {installPhase !== 'idle' && (
                <div className={`install-status ${installPhase}`} role="status">
                  <div className="install-status-icon">
                    {installPhase === 'installing' && <LoaderCircle className="spin" size={20} />}
                    {installPhase === 'success' && <CircleCheck size={20} />}
                    {installPhase === 'error' && <TriangleAlert size={20} />}
                  </div>
                  <div>
                    <strong>
                      {installPhase === 'installing' && t('正在连接并部署探针')}
                      {installPhase === 'success' && t('探针部署完成')}
                      {installPhase === 'error' && t('安装失败')}
                    </strong>
                    <span>
                      {installPhase === 'installing' && t('请保持此窗口开启，通常需要几十秒。')}
                      {installPhase === 'success' && (installHost?.agent_id ? t('探针已连接并开始上报。') : t('正在等待探针首次上报。'))}
                      {installPhase === 'error' && installError}
                    </span>
                  </div>
                  {installPhase === 'installing' && <div className="install-progress"><i /></div>}
                </div>
              )}

              {installHost?.install_logs.length ? (
                <div className="install-log-preview">
                  <span className="muted small">{t('最新安装日志')}</span>
                  {installHost.install_logs.slice(-3).reverse().map((log) => (
                    <pre className={log.ok ? 'ok' : 'failed'} key={`${log.at}-${log.message}`}>
                      {log.message}
                    </pre>
                  ))}
                </div>
              ) : null}

              <div className="filter-tabs">
                {installHost?.has_ssh_password && (
                  <button
                    className={installAuth === 'saved' ? 'active' : ''}
                    disabled={installPhase === 'installing'}
                    onClick={() => setInstallAuth('saved')}
                    type="button"
                  >
                    {t('使用已保存密码')}
                  </button>
                )}
                {installHost?.has_ssh_identity && (
                  <button
                    className={installAuth === 'identity' ? 'active' : ''}
                    disabled={installPhase === 'installing'}
                    onClick={() => setInstallAuth('identity')}
                    type="button"
                  >
                    {t('使用已保存身份文件')}
                  </button>
                )}
                <button
                  className={installAuth === 'password' ? 'active' : ''}
                  disabled={installPhase === 'installing'}
                  onClick={() => setInstallAuth('password')}
                  type="button"
                >
                  {t('临时密码')}
                </button>
                <button
                  className={installAuth === 'key' ? 'active' : ''}
                  disabled={installPhase === 'installing'}
                  onClick={() => setInstallAuth('key')}
                  type="button"
                >
                  {t('密钥登录')}
                </button>
              </div>
              {installAuth === 'saved' && (
                <p className="muted small">{t('将使用该主机已保存的 SSH 密码安装探针。')}</p>
              )}
              {installAuth === 'identity' && (
                <p className="muted small">{t('将使用该主机已保存的 SSH 身份文件安装探针。')}</p>
              )}
              {installAuth === 'password' && (
                <label>
                  {t('SSH 密码')}
                  <input
                    autoComplete="new-password"
                    disabled={installPhase === 'installing'}
                    required
                    type="password"
                    value={installPassword}
                    onChange={(e) => setInstallPassword(e.target.value)}
                    placeholder={t('目标机 root/用户 登录密码')}
                  />
                </label>
              )}
              {installAuth === 'key' && (
                <label>
                  {t('SSH 私钥路径（服务端/容器内）')}
                  <input
                    required
                    value={installKeyPath}
                    onChange={(e) => setInstallKeyPath(e.target.value)}
                    placeholder="/root/.ssh/id_rsa"
                  />
                </label>
              )}
              {installPhase === 'error' && !installError && <div className="banner error">{t('安装失败，请重试')}</div>}

              <div className="manual-install">
                <button
                  aria-expanded={manualInstallOpen}
                  className="manual-install-toggle"
                  disabled={installPhase === 'installing'}
                  onClick={toggleManualInstall}
                  type="button"
                >
                  <span><Terminal size={15} /> {t('手动命令安装')}</span>
                  <ChevronDown className={manualInstallOpen ? 'expanded' : ''} size={17} />
                </button>
                {manualInstallOpen && (
                  <div className="manual-install-content">
                    {tokenLoading && <div className="manual-loading"><LoaderCircle className="spin" size={16} /> {t('正在生成安装命令')}</div>}
                    {manualInstallError && <div className="banner error">{manualInstallError}</div>}
                    {tokenInfo && (
                      <>
                        <label>
                          <span>{t('探针认证 Token')}</span>
                          <span className="copy-help">{t('目标主机上的 Agent 使用它连接当前监控服务，请勿泄露。')}</span>
                          <div className="copy-row">
                            <input className="mono" readOnly value={tokenInfo.agent_token} />
                            <button
                              aria-label={t('复制探针认证 Token')}
                              className="icon-btn"
                              onClick={() => void copyText(tokenInfo.agent_token, 'agent-token')}
                              title={t('复制探针认证 Token')}
                              type="button"
                            >
                              {copiedItem === 'agent-token' ? <CircleCheck size={15} /> : <Copy size={15} />}
                            </button>
                          </div>
                          {copiedItem === 'agent-token' && <span className="copy-feedback">{t('已复制：探针认证 Token')}</span>}
                        </label>
                        <label>
                          <span>{t('探针一键安装命令')}</span>
                          <span className="copy-help">{t('请复制到目标主机的终端执行，用于安装并注册 Agent。')}</span>
                          <div className="copy-row">
                            <textarea className="mono cmd-box" readOnly rows={4} value={tokenInfo.install_command} />
                            <button
                              aria-label={t('复制探针一键安装命令')}
                              className="icon-btn"
                              onClick={() => void copyText(tokenInfo.install_command, 'install-command')}
                              title={t('复制探针一键安装命令')}
                              type="button"
                            >
                              {copiedItem === 'install-command' ? <CircleCheck size={15} /> : <Copy size={15} />}
                            </button>
                          </div>
                          {copiedItem === 'install-command' && <span className="copy-feedback">{t('已复制：探针一键安装命令')}</span>}
                        </label>
                      </>
                    )}
                  </div>
                )}
              </div>
            </div>
            <div className="modal-actions">
              <button className="btn secondary" disabled={installPhase === 'installing'} onClick={closeInstallModal} type="button">
                {installPhase === 'success' ? t('完成') : t('取消')}
              </button>
              {installPhase !== 'success' && (
                <button className="btn primary" disabled={installPhase === 'installing'} type="submit">
                  {installPhase === 'installing' && <LoaderCircle className="spin" size={15} />}
                  {installPhase === 'installing' ? t('安装中') : installPhase === 'error' ? t('重新安装') : t('开始安装')}
                </button>
              )}
            </div>
          </form>
        </div>
      )}
    </div>
  )
}

function SummaryCard({
  icon,
  label,
  value,
  tone,
}: {
  icon: ReactNode
  label: string
  value: number
  tone?: string
}) {
  return (
    <div className={`summary-card ${tone ?? ''}`}>
      <div className="summary-icon-wrap">{icon}</div>
      <div className="summary-info">
        <span>{label}</span>
        <strong>{value}</strong>
      </div>
    </div>
  )
}

async function consumeServerEvents(
  token: string,
  signal: AbortSignal,
  onUnauthorized: () => void,
  onEvent: (event: ServerEvent) => void,
) {
  while (!signal.aborted) {
    try {
      const response = await fetch('/events', {
        headers: { Authorization: `Bearer ${token}` },
        signal,
      })
      if (response.status === 401) {
        onUnauthorized()
        return
      }
      if (!response.ok || !response.body) throw new Error(`event stream returned ${response.status}`)

      const reader = response.body.getReader()
      const decoder = new TextDecoder()
      let buffer = ''
      while (!signal.aborted) {
        const { done, value } = await reader.read()
        if (done) break
        buffer += decoder.decode(value, { stream: true }).replace(/\r\n/g, '\n')
        let boundary = buffer.indexOf('\n\n')
        while (boundary >= 0) {
          const block = buffer.slice(0, boundary)
          buffer = buffer.slice(boundary + 2)
          const data = block
            .split('\n')
            .filter((line) => line.startsWith('data:'))
            .map((line) => line.slice(5).trimStart())
            .join('\n')
          if (data) onEvent(JSON.parse(data) as ServerEvent)
          boundary = buffer.indexOf('\n\n')
        }
      }
    } catch (error) {
      if (signal.aborted) return
      console.warn('event stream disconnected', error)
    }

    await new Promise((resolve) => window.setTimeout(resolve, 1500))
  }
}

function VersionPanel({ token, onUnauthorized }: { token: string; onUnauthorized: () => void }) {
  const { language, t } = useI18n()
  const [catalog, setCatalog] = useState<ReleaseCatalog>()
  const [loading, setLoading] = useState(true)
  const [applyingVersion, setApplyingVersion] = useState<string>()
  const [restarting, setRestarting] = useState(false)
  const [versionError, setVersionError] = useState('')

  async function loadCatalog() {
    setLoading(true)
    setVersionError('')
    try {
      setCatalog(await fetchReleaseCatalog(token, onUnauthorized))
    } catch (error) {
      setVersionError(error instanceof Error ? error.message : t('版本信息加载失败'))
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    void loadCatalog()
    // The token is stable for the authenticated page lifetime.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [token])

  async function switchVersion(version: string) {
    if (!window.confirm(t('确认切换到 {version}？服务会自动重启。', { version }))) return
    setApplyingVersion(version)
    setVersionError('')
    try {
      await applyRelease(version, token, onUnauthorized)
      setRestarting(true)
      const deadline = Date.now() + 90_000
      while (Date.now() < deadline) {
        await new Promise((resolve) => window.setTimeout(resolve, 1200))
        try {
          const response = await fetch('/api/health', { cache: 'no-store' })
          if (response.ok) {
            window.location.reload()
            return
          }
        } catch {
          // The expected restart window temporarily rejects requests.
        }
      }
      setVersionError(t('版本切换失败'))
      setRestarting(false)
    } catch (error) {
      setVersionError(error instanceof Error ? error.message : t('版本切换失败'))
      setApplyingVersion(undefined)
    }
  }

  if (loading && !catalog) {
    return <div className="empty-state"><LoaderCircle className="spin" size={18} /> {t('加载中…')}</div>
  }

  return (
    <section className="version-panel">
      <div className="version-overview">
        <div>
          <span>{t('当前版本')}</span>
          <strong>v{catalog?.current_version ?? '-'}</strong>
        </div>
        <div>
          <span>{t('最新版本')}</span>
          <strong>{catalog?.latest_version ? `v${catalog.latest_version}` : '-'}</strong>
        </div>
        <button className="icon-btn" disabled={loading || restarting} onClick={() => void loadCatalog()} title={t('检查更新')} type="button">
          <RefreshCw className={loading ? 'spin' : ''} size={17} />
        </button>
      </div>

      {!catalog?.managed_updates && (
        <div className="banner error">
          {t('当前部署未启用管理端版本切换，请使用官方 Docker 配置。')}
        </div>
      )}
      {restarting && <div className="version-restarting"><LoaderCircle className="spin" size={18} /> {t('服务正在重启，请稍候…')}</div>}
      {versionError && <div className="banner error">{versionError}</div>}

      <div className="section-head">
        <h3>{t('可用版本')}</h3>
        <span className="muted small">{catalog?.github_repo}</span>
      </div>
      <div className="version-list">
        {catalog?.releases.map((release, index) => {
          const canApply = catalog.managed_updates && Boolean(release.asset_name) && !release.active && !restarting
          const isNewer = index < catalog.releases.findIndex((item) => item.active)
          return (
            <article className={`version-row${release.active ? ' active' : ''}`} key={release.version}>
              <div className="version-main">
                <div className="version-title">
                  <strong>v{release.version}</strong>
                  {release.active && <span className="tag">{t('当前使用')}</span>}
                  {release.installed && !release.active && <span className="tag">{t('已下载')}</span>}
                  {release.prerelease && <span className="tag">{t('预发布')}</span>}
                </div>
                <span>{release.name}</span>
                <span className="muted small">
                  {t('发布时间')}: {release.published_at ? new Date(release.published_at).toLocaleString(language) : '-'}
                  {release.asset_size ? ` · ${formatBytes(release.asset_size)}` : ''}
                </span>
              </div>
              <div className="version-actions">
                <a className="icon-btn" href={release.html_url} rel="noreferrer" target="_blank" title={t('查看 GitHub Release')}>
                  <ExternalLink size={15} />
                </a>
                <button
                  className="btn secondary"
                  disabled={!canApply || Boolean(applyingVersion)}
                  onClick={() => void switchVersion(release.version)}
                  type="button"
                >
                  {applyingVersion === release.version ? <LoaderCircle className="spin" size={15} /> : isNewer ? <Download size={15} /> : <RotateCcw size={15} />}
                  {!release.asset_name
                    ? t('此 Release 缺少当前平台应用包')
                    : isNewer
                      ? t('更新到此版本')
                      : t('回退到此版本')}
                </button>
              </div>
            </article>
          )
        })}
        {catalog?.releases.length === 0 && <div className="empty-inline">{t('暂无可用版本')}</div>}
      </div>
    </section>
  )
}

type HistoryChartDatum = {
  at: Date
  cpu: number
  memory: number
  disk: number
  load: number
  rxRate: number
  txRate: number
}

type HistorySeriesKey = 'cpu' | 'memory' | 'disk' | 'load' | 'rxRate' | 'txRate'

const historySeries: Record<HistoryView, Array<{ key: HistorySeriesKey; label: string; color: string }>> = {
  resources: [
    { key: 'cpu', label: 'CPU', color: 'var(--cpu)' },
    { key: 'memory', label: '内存', color: 'var(--mem)' },
    { key: 'disk', label: '磁盘', color: 'var(--disk)' },
  ],
  network: [
    { key: 'rxRate', label: '接收', color: 'var(--accent)' },
    { key: 'txRate', label: '发送', color: 'var(--ok)' },
  ],
  load: [{ key: 'load', label: '1 分钟负载', color: 'var(--warn)' }],
}

function HostHistoryPanel({
  host,
  token,
  onUnauthorized,
}: {
  host: Host
  token: string
  onUnauthorized: () => void
}) {
  const { t } = useI18n()
  const [range, setRange] = useState<HistoryRange>('1h')
  const [view, setView] = useState<HistoryView>('resources')
  const [history, setHistory] = useState<MetricHistoryResponse>()
  const [loading, setLoading] = useState(true)
  const [historyError, setHistoryError] = useState('')
  const onUnauthorizedRef = useRef(onUnauthorized)
  onUnauthorizedRef.current = onUnauthorized

  useEffect(() => {
    let cancelled = false

    async function loadHistory(initial: boolean) {
      if (initial) {
        setLoading(true)
        setHistory(undefined)
      }
      setHistoryError('')
      try {
        const response = await authFetch(
          `/api/hosts/${host.id}/metrics-history?range=${range}`,
          undefined,
          token,
          () => onUnauthorizedRef.current(),
        )
        if (!response.ok) throw new Error(await readError(response))
        const data = (await response.json()) as MetricHistoryResponse
        if (!cancelled) setHistory(data)
      } catch (err) {
        if (!cancelled) setHistoryError(err instanceof Error ? err.message : t('历史数据加载失败'))
      } finally {
        if (!cancelled) setLoading(false)
      }
    }

    void loadHistory(true)
    const refreshMs = Math.max(5000, Math.min(host.update_interval_seconds * 1000, 60000))
    const timer = window.setInterval(() => void loadHistory(false), refreshMs)
    return () => {
      cancelled = true
      window.clearInterval(timer)
    }
  }, [host.id, host.update_interval_seconds, range, t, token])

  const chartData = useMemo<HistoryChartDatum[]>(() => {
    if (!history) return []
    return history.points.map((point, index, points) => {
      const at = new Date(point.collected_at)
      const previous = points[index - 1]
      const previousAt = previous ? new Date(previous.collected_at) : undefined
      const elapsed = previousAt ? Math.max(1, (at.getTime() - previousAt.getTime()) / 1000) : 1
      const rxDelta = previous ? point.network_rx_bytes - previous.network_rx_bytes : 0
      const txDelta = previous ? point.network_tx_bytes - previous.network_tx_bytes : 0
      return {
        at,
        cpu: point.cpu_percent,
        memory: point.memory_percent,
        disk: point.disk_percent,
        load: point.load_one,
        rxRate: Math.max(0, rxDelta) / elapsed / 1024,
        txRate: Math.max(0, txDelta) / elapsed / 1024,
      }
    })
  }, [history])

  return (
    <section className="detail-section history-section">
      <div className="history-head">
        <h4>{t('历史趋势')}</h4>
        {history && <span className="muted small">{t('{count} 个采样点', { count: history.points.length })}</span>}
      </div>
      <div className="history-controls">
        <div className="filter-tabs">
          {([
            ['1h', t('最近 1h')],
            ['4h', '4h'],
            ['6h', '6h'],
            ['12h', '12h'],
            ['1d', t('1天')],
          ] as const).map(([value, label]) => (
            <button className={range === value ? 'active' : ''} key={value} onClick={() => setRange(value)} type="button">
              {label}
            </button>
          ))}
        </div>
        <div className="filter-tabs history-view-tabs">
          {([
            ['resources', t('资源')],
            ['network', t('网络')],
            ['load', t('负载')],
          ] as const).map(([value, label]) => (
            <button className={view === value ? 'active' : ''} key={value} onClick={() => setView(value)} type="button">
              {label}
            </button>
          ))}
        </div>
      </div>
      {historyError && <div className="banner error">{historyError}</div>}
      {loading ? (
        <div className="history-empty"><LoaderCircle className="spin" size={17} /> {t('加载历史数据')}</div>
      ) : chartData.length > 1 ? (
        <D3HistoryChart data={chartData} view={view} />
      ) : (
        <div className="history-empty">{t('当前时间范围内暂无足够数据')}</div>
      )}
    </section>
  )
}

function D3HistoryChart({ data, view }: { data: HistoryChartDatum[]; view: HistoryView }) {
  const { language, t } = useI18n()
  const containerRef = useRef<HTMLDivElement>(null)
  const [hovered, setHovered] = useState<HistoryChartDatum>()
  const series = historySeries[view]

  useEffect(() => {
    const container = containerRef.current
    if (!container || data.length < 2) return undefined
    const chartContainer: HTMLDivElement = container

    function renderChart() {
      const width = Math.max(chartContainer.clientWidth, 280)
      const height = 260
      const margin = { top: 12, right: 14, bottom: 30, left: 48 }
      const innerWidth = width - margin.left - margin.right
      const innerHeight = height - margin.top - margin.bottom
      d3.select(chartContainer).selectAll('*').remove()

      const svg = d3
        .select(chartContainer)
        .append('svg')
        .attr('viewBox', `0 0 ${width} ${height}`)
        .attr('role', 'img')
        .attr('aria-label', t('主机历史指标趋势图'))

      const chart = svg.append('g').attr('transform', `translate(${margin.left},${margin.top})`)
      const extent = d3.extent(data, (point) => point.at)
      const start = extent[0] ?? new Date()
      const end = extent[1] ?? start
      const xDomain: [Date, Date] =
        start.getTime() === end.getTime()
          ? [new Date(start.getTime() - 30000), new Date(end.getTime() + 30000)]
          : [start, end]
      const x = d3.scaleTime().domain(xDomain).range([0, innerWidth])
      const observedMax =
        d3.max(data, (point) => d3.max(series, (item) => Number(point[item.key])) ?? 0) ?? 0
      const yMax = view === 'resources' ? 100 : Math.max(1, observedMax * 1.1)
      const y = d3.scaleLinear().domain([0, yMax]).nice().range([innerHeight, 0])

      chart
        .append('g')
        .attr('class', 'history-grid')
        .call(d3.axisLeft(y).ticks(5).tickSize(-innerWidth).tickFormat(() => ''))
      chart
        .append('g')
        .attr('class', 'history-axis')
        .attr('transform', `translate(0,${innerHeight})`)
        .call(
          d3
            .axisBottom(x)
            .ticks(width < 500 ? 4 : 7)
            .tickFormat((value) => d3.timeFormat('%H:%M')(value as Date)),
        )
      chart.append('g').attr('class', 'history-axis').call(d3.axisLeft(y).ticks(5))

      for (const item of series) {
        const line = d3
          .line<HistoryChartDatum>()
          .defined((point) => Number.isFinite(point[item.key]))
          .x((point) => x(point.at))
          .y((point) => y(Number(point[item.key])))
        chart
          .append('path')
          .datum(data)
          .attr('class', 'history-line')
          .attr('stroke', item.color)
          .attr('d', line)
      }

      const focus = chart.append('g').style('display', 'none').attr('pointer-events', 'none')
      focus.append('line').attr('class', 'history-focus-line').attr('y1', 0).attr('y2', innerHeight)
      const focusDots = focus
        .selectAll('circle')
        .data(series)
        .join('circle')
        .attr('r', 4)
        .attr('fill', (item) => item.color)
        .attr('stroke', 'var(--bg-elevated)')
        .attr('stroke-width', 2)
      const bisect = d3.bisector<HistoryChartDatum, Date>((point) => point.at).center
      const overlay = chart
        .append('rect')
        .attr('class', 'history-overlay')
        .attr('width', innerWidth)
        .attr('height', innerHeight)

      overlay
        .on('pointerenter', () => focus.style('display', null))
        .on('pointerleave', () => {
          focus.style('display', 'none')
          setHovered(undefined)
        })
        .on('pointermove', (event: PointerEvent) => {
          const [pointerX] = d3.pointer(event, overlay.node())
          const index = Math.max(0, Math.min(data.length - 1, bisect(data, x.invert(pointerX))))
          const point = data[index]
          const focusX = x(point.at)
          focus.attr('transform', `translate(${focusX},0)`)
          focusDots
            .attr('cx', 0)
            .attr('cy', (item) => y(Number(point[item.key])))
          setHovered(point)
        })
    }

    renderChart()
    const observer = new ResizeObserver(renderChart)
    observer.observe(chartContainer)
    return () => {
      observer.disconnect()
      d3.select(chartContainer).selectAll('*').remove()
    }
  }, [data, series, t, view])

  return (
    <div className="history-chart-wrap">
      <div className="history-legend">
        {series.map((item) => <span key={item.key}><i style={{ background: item.color }} />{t(item.label)}</span>)}
      </div>
      <div className="history-chart" ref={containerRef} />
      {hovered && (
        <div className="history-tooltip">
          <strong>{hovered.at.toLocaleString(language)}</strong>
          {series.map((item) => (
            <span key={item.key}>
              <i style={{ background: item.color }} />
              {t(item.label)} {formatHistoryValue(Number(hovered[item.key]), view)}
            </span>
          ))}
        </div>
      )}
    </div>
  )
}

function formatHistoryValue(value: number, view: HistoryView) {
  if (view === 'resources') return `${value.toFixed(2)}%`
  if (view === 'network') return `${value.toFixed(1)} KB/s`
  return value.toFixed(2)
}

function HostMetricSummary({ host }: { host: Host }) {
  const { t } = useI18n()
  if (!host.latest) {
    return <div className="dashboard-host-empty">{t('等待探针上报实时指标')}</div>
  }

  const sample = host.latest!
  const memoryPercent = percent(sample.memory_used_bytes, sample.memory_total_bytes)
  const disk = sample.disks[0]
  const diskPercent = disk ? percent(disk.total_bytes - disk.available_bytes, disk.total_bytes) : 0
  const rxRate =
    sample.network_rx_rate !== undefined ? ` (${formatBytes(sample.network_rx_rate)}/s)` : ''
  const txRate =
    sample.network_tx_rate !== undefined ? ` (${formatBytes(sample.network_tx_rate)}/s)` : ''

  return (
    <div className="dashboard-host-metrics">
      <div className="dashboard-host-resources">
        <MetricBar
          icon={<Cpu size={15} />}
          name="CPU"
          detail={formatCpuDetail(sample.cpu_percent, sample.cpu_cores)}
          value={sample.cpu_percent}
          tone="cpu"
        />
        <MetricBar
          icon={<MemoryStick size={15} />}
          name={t('内存')}
          detail={formatUsageDetail(sample.memory_used_bytes, sample.memory_total_bytes)}
          value={memoryPercent}
          tone="mem"
        />
        <MetricBar
          icon={<HardDrive size={15} />}
          name={t('磁盘')}
          detail={
            disk
              ? formatUsageDetail(disk.total_bytes - disk.available_bytes, disk.total_bytes)
              : t('磁盘 无数据')
          }
          value={diskPercent}
          tone="disk"
        />
      </div>
      <div className="dashboard-host-live">
        <div className="live-stat">
          <span className="live-stat-label">
            <Network size={14} />
            {t('接收')}
          </span>
          <strong className="live-stat-value">
            {formatBytes(sample.network_rx_bytes)}
            {rxRate}
          </strong>
        </div>
        <div className="live-stat">
          <span className="live-stat-label">{t('发送')}</span>
          <strong className="live-stat-value">
            {formatBytes(sample.network_tx_bytes)}
            {txRate}
          </strong>
        </div>
        <div className="live-stat">
          <span className="live-stat-label">{t('负载')}</span>
          <strong className="live-stat-value">{formatLoad(sample.load_average)}</strong>
        </div>
      </div>
    </div>
  )
}

function HostDetailContent({ host, tab }: { host: Host; tab: 'info' | 'load' | 'logs' }) {
  const { language, t } = useI18n()
  const sample = host.latest

  if (tab === 'info') {
    return (
      <div className="host-detail-content">
        <section className="detail-section">
          <h4>{t('连接信息')}</h4>
          <div className="detail-grid">
            <DetailValue label={t('状态')} value={statusLabel(host.status)} />
            <DetailValue label={t('IP / 域名')} value={host.address} mono />
            <DetailValue label={t('地区')} value={host.region || t('未识别')} />
            <DetailValue label="SSH" value={`${host.ssh_user || t('未设置')} @ ${host.ssh_port}`} mono />
            <DetailValue label={t('数据更新')} value={formatUpdateInterval(host.update_interval_seconds)} />
            <DetailValue label={t('SSH 密码')} value={host.has_ssh_password ? t('已保存') : t('未保存')} />
            <DetailValue label={t('SSH 身份文件')} value={host.has_ssh_identity ? t('已保存') : t('未保存')} />
            <DetailValue label={t('最后上报')} value={host.last_seen ? new Date(host.last_seen).toLocaleString(language) : t('从未上报')} />
            <DetailValue label={t('探针 ID')} value={host.agent_id || t('未注册')} mono />
            <DetailValue label={t('创建时间')} value={new Date(host.created_at).toLocaleString(language)} />
          </div>
        </section>

        {sample && (
          <section className="detail-section">
            <h4>{t('系统信息')}</h4>
            <div className="detail-grid">
              <DetailValue label={t('主机名')} value={sample.hostname} mono />
              <DetailValue label={t('操作系统')} value={sample.os} />
              <DetailValue label={t('内核')} value={sample.kernel} mono />
              <DetailValue label={t('逻辑核心')} value={sample.cpu_cores ? `${sample.cpu_cores} ${t('核')}` : t('未知')} />
            </div>
          </section>
        )}

        <section className="detail-section">
          <h4>{t('标签')}</h4>
          {host.tags.length ? (
            <div className="tag-row">{host.tags.map((tag) => <span className="tag" key={tag}>{t(tag)}</span>)}</div>
          ) : (
            <span className="muted small">{t('暂无标签')}</span>
          )}
        </section>
      </div>
    )
  }

  if (tab === 'load') {
    return (
      <div className="host-detail-content">
        {sample ? (
          <>
            <section className="detail-section">
              <h4>{t('系统负载')}</h4>
              <div className="detail-grid">
                <DetailValue label={t('运行时间')} value={formatDuration(sample.uptime_seconds)} />
                <DetailValue label={t('系统负载')} value={formatLoad(sample.load_average)} mono />
                <DetailValue label={t('采集时间')} value={new Date(sample.collected_at).toLocaleString(language)} />
              </div>
            </section>

            <section className="detail-section">
              <h4>{t('资源使用')}</h4>
              <div className="detail-metrics">
                <MetricBar
                  icon={<Cpu size={16} />}
                  name="CPU"
                  detail={formatCpuDetail(sample.cpu_percent, sample.cpu_cores)}
                  value={sample.cpu_percent}
                  tone="cpu"
                />
                <MetricBar
                  icon={<MemoryStick size={16} />}
                  name={t('内存')}
                  detail={formatUsageDetail(sample.memory_used_bytes, sample.memory_total_bytes)}
                  value={percent(sample.memory_used_bytes, sample.memory_total_bytes)}
                  tone="mem"
                />
                <MetricBar
                  name={t('交换区')}
                  detail={formatUsageDetail(sample.swap_used_bytes, sample.swap_total_bytes)}
                  value={percent(sample.swap_used_bytes, sample.swap_total_bytes)}
                  tone="mem"
                />
              </div>
            </section>

            <section className="detail-section">
              <h4>{t('网络流量')}</h4>
              <div className="detail-grid">
                <DetailValue
                  label={t('累计接收')}
                  value={`${formatBytes(sample.network_rx_bytes)}${sample.network_rx_rate !== undefined ? ` (${formatBytes(sample.network_rx_rate)}/s)` : ''}`}
                />
                <DetailValue
                  label={t('累计发送')}
                  value={`${formatBytes(sample.network_tx_bytes)}${sample.network_tx_rate !== undefined ? ` (${formatBytes(sample.network_tx_rate)}/s)` : ''}`}
                />
              </div>
            </section>

            <section className="detail-section">
              <h4>{t('磁盘')}</h4>
              {sample.disks.length ? (
                <div className="detail-disks">
                  {sample.disks.map((disk) => (
                    <div className="detail-disk" key={`${disk.name}-${disk.mount_point}`}>
                      <MetricBar
                        icon={<HardDrive size={16} />}
                        name={`${disk.name} · ${disk.mount_point}`}
                        detail={formatUsageDetail(
                          disk.total_bytes - disk.available_bytes,
                          disk.total_bytes,
                        )}
                        value={percent(disk.total_bytes - disk.available_bytes, disk.total_bytes)}
                        tone="disk"
                      />
                      <span>{formatBytes(disk.available_bytes)} {t('可用空间')}</span>
                    </div>
                  ))}
                </div>
              ) : (
                <div className="empty-inline">{t('暂无磁盘数据')}</div>
              )}
            </section>
          </>
        ) : (
          <div className="empty-inline">{t('探针尚未上报系统数据')}</div>
        )}
      </div>
    )
  }

  // tab === 'logs'
  return (
    <div className="host-detail-content">
      <section className="detail-section">
        <h4>{t('安装日志')}</h4>
        {host.install_logs.length ? (
          <div className="logs">
            {host.install_logs.slice(-10).reverse().map((log) => (
              <pre className={log.ok ? 'ok' : 'failed'} key={`${log.at}-${log.message}`}>
                [{new Date(log.at).toLocaleString()}] {log.message}
              </pre>
            ))}
          </div>
        ) : (
          <span className="muted small">{t('暂无安装日志')}</span>
        )}
      </section>
    </div>
  )
}

function DetailValue({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="detail-value">
      <span>{label}</span>
      <strong className={mono ? 'mono' : ''}>{value}</strong>
    </div>
  )
}

function formatUpdateInterval(seconds: number) {
  if (seconds >= 60 && seconds % 60 === 0) return translate('每 {count} 分钟', { count: seconds / 60 })
  return translate('每 {count} 秒', { count: seconds })
}
