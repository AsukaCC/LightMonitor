import { Link2, LoaderCircle, RefreshCw, Save, Trash2, Upload, X } from 'lucide-react'
import { useRef, useState } from 'react'
import type { FormEvent } from 'react'
import { assignSshKeyHosts, deleteSshKey, updateSshKey, uploadSshKey } from '../api'
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
  onHostsReload: () => Promise<void>
}

export function SshKeyPanel({ keys, hosts, loading, error, token, onUnauthorized, onReload, onHostsReload }: Props) {
  const { language, t } = useI18n()
  const [name, setName] = useState('')
  const [file, setFile] = useState<File>()
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState('')
  const [replaceId, setReplaceId] = useState<string>()
  const [bindingKeyId, setBindingKeyId] = useState<string>()
  const [bindingHostIds, setBindingHostIds] = useState<string[]>([])
  const replaceInput = useRef<HTMLInputElement>(null)

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

  function editBindings(key: SshKey) {
    setBindingKeyId(key.id)
    setBindingHostIds(key.host_ids ?? [])
    setMessage('')
  }

  async function saveBindings(key: SshKey) {
    setBusy(true)
    setMessage('')
    try {
      await assignSshKeyHosts(key.id, bindingHostIds, token, onUnauthorized)
      await Promise.all([onReload(), onHostsReload()])
      setBindingKeyId(undefined)
    } catch (err) {
      setMessage(err instanceof Error ? err.message : t('主机关联保存失败'))
    } finally {
      setBusy(false)
    }
  }

  const assignableHosts = hosts.filter((host) => !host.is_system)

  return (
    <section className="panel ssh-key-panel">
      <div className="section-head">
        <div>
          <h3>{t('服务器 SSH 密钥')}</h3>
          <p className="muted small">{t('密钥保存在 LightMonitor 数据卷，仅用于服务器发起 SSH 连接。')}</p>
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
          {busy && !replaceId ? <LoaderCircle className="spin" size={15} /> : <Upload size={15} />}
          {t('上传密钥')}
        </button>
      </form>

      {(error || message) && <div className="banner error">{message || error}</div>}
      <div className="ssh-key-list">
        {loading && keys.length === 0 && <div className="empty-state"><LoaderCircle className="spin" size={18} /> {t('加载中…')}</div>}
        {keys.map((key) => (
          <article className="ssh-key-row" key={key.id}>
            <div className="ssh-key-row-main">
              <div className="ssh-key-info">
                <strong>{key.name}</strong>
                <span className="muted small">
                  {formatBytes(key.size_bytes)} · {new Date(key.updated_at).toLocaleString(language)}
                </span>
                <span className="muted small">
                  {(key.host_names ?? []).length > 0
                    ? t('已关联主机：{names}', { names: key.host_names.join(', ') })
                    : t('未关联主机')}
                </span>
              </div>
              <div className="ssh-key-actions">
                <button className="btn secondary" disabled={busy} onClick={() => editBindings(key)} type="button">
                  <Link2 size={15} /> {t('关联主机')}
                </button>
                <label className="btn secondary file-picker">
                  {replaceId === key.id ? <LoaderCircle className="spin" size={15} /> : <Upload size={15} />}
                  {t('更新')}
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
            </div>
            {bindingKeyId === key.id && (
              <div className="ssh-key-bindings">
                <div className="ssh-key-binding-head">
                  <strong>{t('选择使用此密钥的主机')}</strong>
                  <button className="icon-btn" disabled={busy} onClick={() => setBindingKeyId(undefined)} title={t('关闭')} type="button">
                    <X size={15} />
                  </button>
                </div>
                <div className="ssh-key-host-list">
                  {assignableHosts.map((host) => (
                    <label className="checkbox-row" key={host.id}>
                      <input
                        checked={bindingHostIds.includes(host.id)}
                        disabled={busy}
                        onChange={(event) => setBindingHostIds((current) => (
                          event.target.checked
                            ? [...current, host.id]
                            : current.filter((id) => id !== host.id)
                        ))}
                        type="checkbox"
                      />
                      <span>{host.name}</span>
                      <span className="muted small mono">{host.address}</span>
                    </label>
                  ))}
                  {assignableHosts.length === 0 && <span className="muted small">{t('暂无可关联主机')}</span>}
                </div>
                <div className="ssh-key-binding-actions">
                  <button className="btn primary" disabled={busy} onClick={() => void saveBindings(key)} type="button">
                    {busy ? <LoaderCircle className="spin" size={15} /> : <Save size={15} />}
                    {t('保存关联')}
                  </button>
                </div>
              </div>
            )}
          </article>
        ))}
        {!loading && keys.length === 0 && <div className="empty-inline">{t('暂无已上传的 SSH 密钥')}</div>}
      </div>
    </section>
  )
}
