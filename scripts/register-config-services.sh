#!/usr/bin/with-contenv bash
set -euo pipefail

SERVICES_DIR="${CUSTOM_SERVICES_DIR:-/config/custom-services.d}"
RUNTIME_DIR="/run/service"

if [[ ! -d "${SERVICES_DIR}" ]]; then
    echo "[custom-services] ${SERVICES_DIR} does not exist, skipping"
    exit 0
fi

for _ in $(seq 1 50); do
    [[ -d "${RUNTIME_DIR}" ]] && break
    sleep 0.1
done

if [[ ! -d "${RUNTIME_DIR}" ]]; then
    echo "[custom-services] ${RUNTIME_DIR} is not available, skipping"
    exit 0
fi

registered=0
seen=" "

for runtime_link in "${RUNTIME_DIR}"/*; do
    [[ -L "${runtime_link}" ]] || continue
    target="$(readlink "${runtime_link}")"
    case "${target}" in
        "${SERVICES_DIR}"/*)
            if [[ ! -e "${target}" ]]; then
                echo "[custom-services] removing stale $(basename "${runtime_link}") -> ${target}"
                rm -f "${runtime_link}"
            fi
            ;;
    esac
done

for service_dir in "${SERVICES_DIR}"/*; do
    [[ -d "${service_dir}" ]] || continue

    service_name="$(basename "${service_dir}")"
    run_file="${service_dir}/run"

    if [[ "${service_name}" == .* ]]; then
        continue
    fi

    if [[ "${seen}" == *" ${service_name} "* ]]; then
        echo "[custom-services] skipping duplicate ${service_name}"
        continue
    fi

    if [[ ! -x "${run_file}" ]]; then
        echo "[custom-services] skipping ${service_name}: missing executable run file"
        continue
    fi

    # Native oneshots register before legacy-services starts. Reserve image
    # service names even while their /run/service links do not exist yet.
    if [[ -d "/etc/services.d/$service_name" || -d "/etc/s6-overlay/s6-rc.d/$service_name" ]]; then
        echo "[custom-services] skipping $service_name: reserved by the image"
        continue
    fi

    runtime_path="${RUNTIME_DIR}/${service_name}"
    if [[ -e "${runtime_path}" && ! -L "${runtime_path}" ]]; then
        echo "[custom-services] skipping ${service_name}: ${runtime_path} already exists and is not a symlink"
        continue
    fi

    if [[ -L "${runtime_path}" ]]; then
        current_target="$(readlink "${runtime_path}")"
        if [[ "${current_target}" != "${service_dir}" && "${current_target}" != "${SERVICES_DIR}"/* ]]; then
            echo "[custom-services] skipping ${service_name}: ${runtime_path} is owned by ${current_target}"
            continue
        fi
    fi

    ln -sfn "${service_dir}" "${runtime_path}"
    echo "[custom-services] registered ${runtime_path} -> ${service_dir}"
    seen="${seen}${service_name} "
    registered=$((registered + 1))
done

if command -v s6-svscanctl >/dev/null 2>&1; then
    s6-svscanctl -a "${RUNTIME_DIR}" || true
fi

echo "[custom-services] registered ${registered} service(s) from ${SERVICES_DIR}"
