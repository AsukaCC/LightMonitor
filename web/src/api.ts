import type { Host, PublicHost, ReleaseCatalog, SshKey } from './types'
import { readError } from './utils'
import { translate } from './i18n'

export const tokenStorageKey = 'lightmonitor.adminToken'
export const userStorageKey = 'lightmonitor.adminUser'

export async function fetchPublicHosts(): Promise<PublicHost[]> {
  const response = await fetch('/api/public/hosts')
  if (!response.ok) throw new Error(await readError(response))
  return (await response.json()) as PublicHost[]
}

export async function authFetch(
  input: RequestInfo | URL,
  init: RequestInit = {},
  token: string,
  onUnauthorized?: () => void,
) {
  const headers = new Headers(init.headers)
  headers.set('Authorization', `Bearer ${token}`)
  const response = await fetch(input, { ...init, headers })
  if (response.status === 401) {
    onUnauthorized?.()
    throw new Error(translate('登录已过期，请重新登录'))
  }
  return response
}

export async function login(username: string, password: string) {
  const response = await fetch('/api/auth/login', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username, password }),
  })
  if (response.status === 401) throw new Error(translate('账户或密码错误'))
  if (!response.ok) throw new Error(await readError(response))
  return (await response.json()) as { token: string; username: string }
}

export async function fetchSession(token: string) {
  const response = await authFetch('/api/auth/session', undefined, token)
  if (!response.ok) throw new Error(await readError(response))
  return (await response.json()) as { username: string }
}

export async function fetchHosts(token: string, onUnauthorized?: () => void) {
  const response = await authFetch('/api/hosts', undefined, token, onUnauthorized)
  if (!response.ok) throw new Error(await readError(response))
  return (await response.json()) as Host[]
}

export async function addHostDomain(
  hostId: string,
  domain: string,
  token: string,
  onUnauthorized?: () => void,
) {
  const response = await authFetch(`/api/hosts/${encodeURIComponent(hostId)}/domains`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ domain }),
  }, token, onUnauthorized)
  if (!response.ok) throw new Error(await readError(response))
  return (await response.json()) as Host
}

export async function deleteHostDomain(
  hostId: string,
  domainId: string,
  token: string,
  onUnauthorized?: () => void,
) {
  const response = await authFetch(
    `/api/hosts/${encodeURIComponent(hostId)}/domains/${encodeURIComponent(domainId)}`,
    { method: 'DELETE' },
    token,
    onUnauthorized,
  )
  if (!response.ok) throw new Error(await readError(response))
  return (await response.json()) as Host
}

export async function probeHost(hostId: string, token: string, onUnauthorized?: () => void) {
  const response = await authFetch(`/api/hosts/${encodeURIComponent(hostId)}/probe`, {
    method: 'POST',
  }, token, onUnauthorized)
  if (!response.ok) throw new Error(await readError(response))
  return (await response.json()) as Host
}

export async function logout(token: string) {
  await authFetch('/api/auth/logout', { method: 'POST' }, token).catch(() => undefined)
}

export async function fetchReleaseCatalog(token: string, onUnauthorized?: () => void) {
  const response = await authFetch('/api/system/releases', undefined, token, onUnauthorized)
  if (!response.ok) throw new Error(await readError(response))
  return (await response.json()) as ReleaseCatalog
}

export async function applyRelease(version: string, token: string, onUnauthorized?: () => void) {
  const response = await authFetch('/api/system/releases/apply', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ version }),
  }, token, onUnauthorized)
  if (!response.ok) throw new Error(await readError(response))
  return (await response.json()) as { version: string; restarting: boolean }
}
export async function deleteDownloadedRelease(version: string, token: string, onUnauthorized?: () => void) {
  const response = await authFetch(`/api/system/releases/${encodeURIComponent(version)}`, {
    method: 'DELETE',
  }, token, onUnauthorized)
  if (!response.ok) throw new Error(await readError(response))
}

export async function fetchSshKeys(token: string, onUnauthorized?: () => void) {
  const response = await authFetch('/api/ssh-keys', undefined, token, onUnauthorized)
  if (!response.ok) throw new Error(await readError(response))
  return (await response.json()) as SshKey[]
}

function sshKeyForm(file: File, name?: string) {
  const body = new FormData()
  if (name?.trim()) body.append('name', name.trim())
  body.append('file', file, file.name)
  return body
}

export async function uploadSshKey(file: File, name: string, token: string, onUnauthorized?: () => void) {
  const response = await authFetch('/api/ssh-keys', {
    method: 'POST',
    body: sshKeyForm(file, name),
  }, token, onUnauthorized)
  if (!response.ok) throw new Error(await readError(response))
  return (await response.json()) as SshKey
}

export async function updateSshKey(id: string, file: File, name: string | undefined, token: string, onUnauthorized?: () => void) {
  const response = await authFetch(`/api/ssh-keys/${encodeURIComponent(id)}`, {
    method: 'PUT',
    body: sshKeyForm(file, name),
  }, token, onUnauthorized)
  if (!response.ok) throw new Error(await readError(response))
  return (await response.json()) as SshKey
}

export async function deleteSshKey(id: string, token: string, onUnauthorized?: () => void) {
  const response = await authFetch(`/api/ssh-keys/${encodeURIComponent(id)}`, {
    method: 'DELETE',
  }, token, onUnauthorized)
  if (!response.ok) throw new Error(await readError(response))
}
