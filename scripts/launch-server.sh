#!/bin/sh

set -u

data_dir="${LIGHTMONITOR_DATA_DIR:-/app/data}"
versions_dir="${LIGHTMONITOR_VERSIONS_DIR:-${data_dir}/versions}"
bundled_dir="${LIGHTMONITOR_BUNDLED_DIR:-/app/bundled}"
active_file="${data_dir}/active-version"
previous_file="${data_dir}/previous-version"
imports_dir="${data_dir}/.bundled-imports"
default_version="$(sed 's/^v//' "${bundled_dir}/VERSION")"
import_marker="${imports_dir}/${default_version}"

mkdir -p "${data_dir}" "${versions_dir}" "${imports_dir}"

is_safe_version() {
  case "$1" in
    ""|"."|".."|*[!A-Za-z0-9._-]*) return 1 ;;
    *) return 0 ;;
  esac
}

is_valid_runtime_dir() {
  [ -x "$1/lightmonitor-server" ] && [ -f "$1/web/index.html" ]
}

is_valid_version() {
  is_safe_version "$1" && is_valid_runtime_dir "${versions_dir}/$1"
}

write_pointer() {
  pointer_path="$1"
  pointer_value="$2"
  pointer_temp="${pointer_path}.tmp.$$"
  printf '%s\n' "${pointer_value}" > "${pointer_temp}" && mv -f "${pointer_temp}" "${pointer_path}"
}

copy_bundled_runtime() {
  staging_dir="${versions_dir}/.bundled-${default_version}-$$"
  destination_dir="${versions_dir}/${default_version}"
  rm -rf "${staging_dir}"
  mkdir -p "${staging_dir}" || return 1
  if ! cp -R "${bundled_dir}/." "${staging_dir}/"; then
    rm -rf "${staging_dir}"
    return 1
  fi
  if ! is_valid_runtime_dir "${staging_dir}"; then
    echo "Bundled LightMonitor ${default_version} is incomplete." >&2
    rm -rf "${staging_dir}"
    return 1
  fi
  rm -rf "${destination_dir}"
  mv "${staging_dir}" "${destination_dir}"
}

import_bundled_once() {
  if [ -f "${import_marker}" ]; then
    return 0
  fi
  if ! is_valid_version "${default_version}"; then
    copy_bundled_runtime || return 1
  fi
  marker_temp="${import_marker}.tmp.$$"
  printf '%s\n' "${default_version}" > "${marker_temp}" && mv -f "${marker_temp}" "${import_marker}"
}

find_fallback_version() {
  excluded_version="$1"
  if [ "${default_version}" != "${excluded_version}" ] && is_valid_version "${default_version}"; then
    printf '%s\n' "${default_version}"
    return 0
  fi
  for candidate_dir in "${versions_dir}"/*; do
    [ -d "${candidate_dir}" ] || continue
    candidate_version="$(basename "${candidate_dir}")"
    [ "${candidate_version}" != "${excluded_version}" ] || continue
    case "${candidate_version}" in .*) continue ;; esac
    if is_valid_version "${candidate_version}"; then
      printf '%s\n' "${candidate_version}"
      return 0
    fi
  done
  return 1
}

select_runtime() {
  requested_runtime="$1"
  if is_valid_version "${requested_runtime}"; then
    selected_version="${requested_runtime}"
  else
    selected_version="$(find_fallback_version "" || true)"
    if [ -z "${selected_version}" ]; then
      echo "No installed LightMonitor runtime is available; restoring bundled ${default_version}." >&2
      copy_bundled_runtime || return 1
      selected_version="${default_version}"
    fi
  fi
  runtime_dir="${versions_dir}/${selected_version}"
}

forward_signal() {
  kill -TERM "${server_pid}" 2>/dev/null || true
  wait "${server_pid}" 2>/dev/null || true
  exit 143
}

if ! is_safe_version "${default_version}"; then
  echo "Invalid bundled LightMonitor version: ${default_version}" >&2
  exit 1
fi

if ! import_bundled_once; then
  echo "Failed to import bundled LightMonitor ${default_version}." >&2
  exit 1
fi

while true; do
  requested_version="${default_version}"
  if [ -f "${active_file}" ]; then
    requested_version="$(sed 's/^v//' "${active_file}")"
  fi
  if ! is_safe_version "${requested_version}"; then
    requested_version="${default_version}"
  fi
  if ! select_runtime "${requested_version}"; then
    exit 1
  fi
  if [ "${selected_version}" != "${requested_version}" ]; then
    write_pointer "${active_file}" "${selected_version}" || exit 1
  fi

  export LIGHTMONITOR_WEB_DIR="${runtime_dir}/web"
  export LIGHTMONITOR_RUNNING_VERSION="${selected_version}"
  export LIGHTMONITOR_RUNTIME_DIR="${runtime_dir}"
  export LIGHTMONITOR_BUNDLED_VERSION="${default_version}"
  "${runtime_dir}/lightmonitor-server" &
  server_pid=$!
  trap forward_signal TERM INT
  wait "${server_pid}"
  exit_code=$?
  trap - TERM INT

  if [ "${exit_code}" -eq 75 ]; then
    continue
  fi

  fallback_version=""
  if [ -f "${previous_file}" ]; then
    previous_version="$(sed 's/^v//' "${previous_file}")"
    if [ "${previous_version}" != "${selected_version}" ] && is_valid_version "${previous_version}"; then
      fallback_version="${previous_version}"
    fi
  fi
  if [ -z "${fallback_version}" ]; then
    fallback_version="$(find_fallback_version "${selected_version}" || true)"
  fi
  rm -f "${previous_file}"

  if [ -n "${fallback_version}" ]; then
    write_pointer "${active_file}" "${fallback_version}" || exit 1
    echo "LightMonitor ${selected_version} exited with ${exit_code}; restored ${fallback_version}." >&2
    continue
  fi

  exit "${exit_code}"
done
