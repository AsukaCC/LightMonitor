#!/bin/sh

set -u

data_dir="${LIGHTMONITOR_DATA_DIR:-/app/data}"
versions_dir="${LIGHTMONITOR_VERSIONS_DIR:-${data_dir}/versions}"
bundled_dir="${LIGHTMONITOR_BUNDLED_DIR:-/app/bundled}"
active_file="${data_dir}/active-version"
previous_file="${data_dir}/previous-version"
default_version="$(sed 's/^v//' "${bundled_dir}/VERSION")"

mkdir -p "${data_dir}" "${versions_dir}"

is_valid_version() {
  [ -x "${versions_dir}/$1/lightmonitor-server" ] &&
    [ -f "${versions_dir}/$1/web/index.html" ]
}

select_runtime() {
  selected_version="$1"
  if [ "${selected_version}" != "${default_version}" ] && is_valid_version "${selected_version}"; then
    runtime_dir="${versions_dir}/${selected_version}"
  else
    selected_version="${default_version}"
    runtime_dir="${bundled_dir}"
  fi
}

forward_signal() {
  kill -TERM "${server_pid}" 2>/dev/null || true
  wait "${server_pid}" 2>/dev/null || true
  exit 143
}

while true; do
  requested_version="${default_version}"
  if [ -f "${active_file}" ]; then
    requested_version="$(sed 's/^v//' "${active_file}")"
  fi
  select_runtime "${requested_version}"

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

  if [ "${selected_version}" != "${default_version}" ] && [ -f "${previous_file}" ]; then
    previous_version="$(sed 's/^v//' "${previous_file}")"
    if [ "${previous_version}" = "${default_version}" ] || is_valid_version "${previous_version}"; then
      printf '%s\n' "${previous_version}" > "${active_file}"
      rm -f "${previous_file}"
      echo "LightMonitor ${selected_version} exited with ${exit_code}; restored ${previous_version}." >&2
      continue
    fi
  fi

  exit "${exit_code}"
done
