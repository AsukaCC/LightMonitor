import { Download, KeyRound, LoaderCircle, RefreshCw, Server, Trash2, Upload } from 'lucide-react'
import { useRef, useState } from 'react'
import type { FormEvent } from 'react'
import { deleteSshKey, downloadSshKey, updateSshKey, uploadSshKey } from '../api'
import { useI18n } from '../i18n'
import type { Host, SshKey } from '../types'
import { formatBytes } from '../utils'

type Props = {
  keys: SshKey[]
  hosts: Host[]
  loading: boolean
  error: string
  token: string
  onUnauthorized: () => void
  onReload: () => Promise<void>
}

export function SshKeyPanel({ keys, hosts, loading, error, token, onUnauthorized, onReload }: Props) {
  const { language, t } = useI18n()
  const [name, setName] = useState('')
  const [file, setFile] = useState<File>()
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState('')
  const [replaceId, setReplaceId] = useState<string>()
  const [downloadId, setDownloadId] = useState<string>()
  const replaceInput = useRef<HTMLInputElement>(null)
  const managedHosts = hosts.filter((host) => !host.is_system)

  async function submitUpload(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!file) {
      setMessage(t('请选择 SSH 私钥文件'))
      return
    }
    setBusy(true)
    setMessage('')
    try {
      await uploadSshKey(file, name, token, onUnauthorized)
      setName('')
      setFile(undefined)
      const input = event.currentTarget.elements.namedItem('ssh-key-file') as HTMLInputElement | null
      if (input) input.value = ''
      await onReload()
    } catch (err) {
      setMessage(err instanceof Error ? err.message : t('密钥上传失败'))
    } finally {
      setBusy(false)
    }
  }

  async function replaceKey(id: string, nextFile?: File) {
    if (!nextFile) return
    setBusy(true)
    setReplaceId(id)
    setMessage('')
    try {
      await updateSshKey(id, nextFile, undefined, token, onUnauthorized)
      await onReload()
    } catch (err) {
      setMessage(err instanceof Error ? err.message : t('密钥更新失败'))
    } finally {
      setReplaceId(undefined)
      setBusy(false)
      if (replaceInput.current) replaceInput.current.value = ''
    }
  }

  async function saveKeyFile(key: SshKey) {
    setBusy(true)
    setDownloadId(key.id)
    setMessage('')
    try {
      const { blob, fileName } = await downloadSshKey(key.id, token, onUnauthorized)
      const url = URL.createObjectURL(blob)
      const anchor = document.createElement('a')
      anchor.href = url
      anchor.download = fileName
      document.body.appendChild(anchor)
      anchor.click()
      anchor.remove()
      URL.revokeObjectURL(url)
    } catch (err) {
      setMessage(err instanceof Error ? err.message : t('密钥下载失败'))
    } finally {
      setDownloadId(undefined)
      setBusy(false)
    }
  }

  async function removeKey(key: SshKey) {
    if (key.in_use || !window.confirm(t('确认删除密钥 {name}？', { name: key.name }))) return
    setBusy(true)
    setMessage('')
    try {
      await deleteSshKey(key.id, token, onUnauthorized)
      await onReload()
    } catch (err) {
      setMessage(err instanceof Error ? err.message : t('密钥删除失败'))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="panel ssh-key-panel">
      <div className="section-head">
        <div>
          <h3>{t('服务器 SSH 密钥')}</h3>
          <p className="muted small">{t('管理已上传的私钥文件；主机关联请在新增或编辑主机中设置。')}</p>
        </div>
        <button className="icon-btn" disabled={loading || busy} onClick={() => void onReload()} title={t('刷新')} type="button">
          <RefreshCw className={loading ? 'spin' : ''} size={16} />
        </button>
      </div>

      <form className="ssh-key-upload" onSubmit={submitUpload}>
        <label>
          {t('密钥名称')}
          <input maxLength={128} required value={name} onChange={(event) => setName(event.target.value)} placeholder={t('例如：生产环境')} />
        </label>
        <label>
          {t('私钥文件')}
          <input
            accept=".pem,.key,.pub,application/octet-stream,text/plain"
            id="ssh-key-file"
            name="ssh-key-file"
            required
            type="file"
            onChange={(event) => setFile(event.target.files?.[0])}
          />
        </label>
        <button className="btn primary" disabled={busy} type="submit">
          {busy && !replaceId && !downloadId ? <LoaderCircle className="spin" size={15} /> : <Upload size={15} />}
          {t('上传密钥')}
        </button>
      </form>

      {(error || message) && <div className="banner error">{message || error}</div>}

      <div className="ssh-key-section-head">
        <h4>{t('密钥文件')}</h4>
        <span className="muted small">{t('共 {count} 个', { count: keys.length })}</span>
      </div>
      <div className="ssh-key-list">
        {loading && keys.length === 0 && <div className="empty-state"><LoaderCircle className="spin" size={18} /> {t('加载中…')}</div>}
        {keys.map((key) => (
          <article className="ssh-key-row" key={key.id}>
            <div className="ssh-key-info">
              <strong>{key.name}</strong>
              <span className="muted small">
                {formatBytes(key.size_bytes)} · {new Date(key.updated_at).toLocaleString(language)}
                {key.in_use ? ` · ${t('正在使用')}` : ''}
              </span>
            </div>
            <div className="ssh-key-actions">
              <button
                className="icon-btn"
                disabled={busy}
                onClick={() => void saveKeyFile(key)}
                title={t('下载密钥')}
                type="button"
              >
                {downloadId === key.id ? <LoaderCircle className="spin" size={15} /> : <Download size={15} />}
              </button>
              <label className="btn secondary file-picker">
                {replaceId === key.id ? <LoaderCircle className="spin" size={15} /> : <Upload size={15} />}
                {t('替换')}
                <input
                  ref={replaceId === key.id ? replaceInput : undefined}
                  accept=".pem,.key,.pub,application/octet-stream,text/plain"
                  disabled={busy}
                  type="file"
                  onChange={(event) => void replaceKey(key.id, event.target.files?.[0])}
                />
              </label>
              <button className="icon-btn danger" disabled={busy || key.in_use} onClick={() => void removeKey(key)} title={key.in_use ? t('密钥正在使用，不能删除') : t('删除')} type="button">
                <Trash2 size={15} />
              </button>
            </div>
          </article>
        ))}
        {!loading && keys.length === 0 && <div className="empty-inline">{t('暂无已上传的 SSH 密钥')}</div>}
      </div>

      <div className="ssh-host-key-section">
        <div className="ssh-key-section-head">
          <div>
            <h4>{t('服务器与密钥')}</h4>
            <p className="muted small">{t('主机使用的密钥由主机登录方式决定。')}</p>
          </div>
          <span className="muted small">{t('共 {count} 台', { count: managedHosts.length })}</span>
        </div>
        <div className="ssh-host-key-list">
          {managedHosts.map((host) => (
            <div className="ssh-host-key-row" key={host.id}>
              <div className="ssh-host-key-host">
                <Server size={17} />
                <div>
                  <strong>{host.name}</strong>
                  <span className="muted small mono">{host.address}</span>
                </div>
              </div>
              <div className={`ssh-host-key-value ${host.ssh_auth_type === 'key' ? 'key' : 'password'}`}>
                {host.ssh_auth_type === 'key' ? <KeyRound size={16} /> : null}
                <div>
                  <span className="muted small">{host.ssh_auth_type === 'key' ? t('SSH 密钥') : t('连接方式')}</span>
                  <strong>
                    {host.ssh_auth_type === 'key'
                      ? (host.ssh_key_name || t('密钥不可用'))
                      : t('账户密码登录')}
                  </strong>
                </div>
              </div>
            </div>
          ))}
          {managedHosts.length === 0 && <div className="empty-inline">{t('暂无可展示的服务器')}</div>}
        </div>
      </div>
    </section>
  )
}
