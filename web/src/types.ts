import { translate } from './i18n'

export type HostStatus = 'pending' | 'installing' | 'online' | 'warning' | 'offline' | 'error'

export type DiskSample = {
  name: string
  mount_point: string
  total_bytes: number
  available_bytes: number
}

export type SystemSample = {
  hostname: string
  os: string
  kernel: string
  uptime_seconds: number
  cpu_cores: number
  cpu_percent: number
  memory_total_bytes: number
  memory_used_bytes: number
  swap_total_bytes: number
  swap_used_bytes: number
  load_average: [number, number, number]
  network_rx_bytes: number
  network_tx_bytes: number
  network_rx_rate?: number
  network_tx_rate?: number
  disks: DiskSample[]
  collected_at: string
}

export type InstallLog = {
  at: string
  ok: boolean
  message: string
}

export type MetricHistoryPoint = {
  collected_at: string
  cpu_percent: number
  memory_percent: number
  disk_percent: number
  load_one: number
  network_rx_bytes: number
  network_tx_bytes: number
  network_rx_rate?: number
  network_tx_rate?: number
}

export type MetricHistoryResponse = {
  range: string
  points: MetricHistoryPoint[]
}

export type Host = {
  id: string
  is_system: boolean
  name: string
  address: string
  region: string
  ssh_user: string
  ssh_port: number
  update_interval_seconds: number
  has_ssh_password: boolean
  has_ssh_identity: boolean
  tags: string[]
  status: HostStatus
  agent_id?: string
  latest?: SystemSample
  last_seen?: string
  install_logs: InstallLog[]
  created_at: string
}

export type SshKey = {
  id: string
  name: string
  size_bytes: number
  updated_at: string
  in_use: boolean
}

export type PublicMetrics = {
  cpu_cores: number
  cpu_percent: number
  memory_used_bytes: number
  memory_total_bytes: number
  memory_percent: number
  disk_used_bytes: number
  disk_total_bytes: number
  disk_percent: number
  load_average: [number, number, number]
  uptime_seconds: number
  network_rx_rate: number
  network_tx_rate: number
}

export type PublicHost = {
  id: string
  name: string
  region: string
  tags: string[]
  status: HostStatus
  metrics?: PublicMetrics
  last_seen?: string
}

export type ServerEvent =
  | { type: 'host_updated'; host: Host }
  | { type: 'hosts_deleted'; host_ids: string[] }
  | { type: 'install_log'; host_id: string; log: InstallLog }

export type HostForm = {
  name: string
  address: string
  ssh_user: string
  ssh_port: string
  ssh_password: string
  clear_ssh_password: boolean
  tags: string
}

export type ThemeMode = 'light' | 'dark'

const statusKeys: Record<HostStatus, string> = {
  pending: '待连接', installing: '安装中', online: '在线', warning: '告警', offline: '离线', error: '异常',
}

export function statusLabel(status: HostStatus) {
  return translate(statusKeys[status])
}

export type AppRelease = {
  version: string
  name: string
  published_at?: string
  html_url: string
  prerelease: boolean
  installed: boolean
  active: boolean
  asset_name?: string
  asset_size?: number
  can_delete: boolean
}

export type ReleaseCatalog = {
  current_version: string
  latest_version?: string
  github_repo: string
  managed_updates: boolean
  platform_asset?: string
  releases: AppRelease[]
}
