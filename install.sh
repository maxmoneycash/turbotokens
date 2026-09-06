#!/bin/sh
# Install the latest (or a pinned) turbotokens release.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/maxmoneycash/turbotokens/main/install.sh | sh
#
# Environment:
#   TURBOTOKENS_VERSION      pin a tag, e.g. "v1.0.0" (default: latest release)
#   TURBOTOKENS_INSTALL_DIR  install directory (default: /usr/local/bin, or ~/.local/bin)
set -eu

REPO="maxmoneycash/turbotokens"
BINARY="turbotokens"

info() {
    printf '%s\n' "$*"
}

err() {
    printf 'error: %s\n' "$*" >&2
}

detect_platform() {
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Darwin) os_part="macos" ;;
        Linux)  os_part="linux" ;;
        *) err "unsupported OS: $os"; return 1 ;;
    esac

    case "$arch" in
        arm64|aarch64) arch_part="arm64" ;;
        x86_64|amd64)  arch_part="x64" ;;
        *) err "unsupported architecture: $arch"; return 1 ;;
    esac

    printf '%s-%s' "$os_part" "$arch_part"
}

resolve_install_dir() {
    if [ -n "${TURBOTOKENS_INSTALL_DIR:-}" ]; then
        printf '%s' "$TURBOTOKENS_INSTALL_DIR"
        return 0
    fi
    if [ -w /usr/local/bin ]; then
        printf '%s' "/usr/local/bin"
    else
        printf '%s' "${HOME}/.local/bin"
    fi
}

download_url() {
    platform="$1"
    asset="${BINARY}-${platform}.tar.gz"
    if [ -n "${TURBOTOKENS_VERSION:-}" ]; then
        printf 'https://github.com/%s/releases/download/%s/%s' "$REPO" "$TURBOTOKENS_VERSION" "$asset"
    else
        printf 'https://github.com/%s/releases/latest/download/%s' "$REPO" "$asset"
    fi
}

main() {
    platform="$(detect_platform)"
    install_dir="$(resolve_install_dir)"
    url="$(download_url "$platform")"

    if [ -n "${TURBOTOKENS_VERSION:-}" ]; then
        info "Installing turbotokens ${TURBOTOKENS_VERSION} (${platform})"
    else
        info "Installing latest turbotokens (${platform})"
    fi
    info "Downloading ${url}"

    tmp_dir="$(mktemp -d)"
    trap 'rm -rf "$tmp_dir"' EXIT

    if ! curl -fsSL "$url" -o "${tmp_dir}/turbotokens.tar.gz"; then
        err "download failed (does a release exist for ${TURBOTOKENS_VERSION:-latest} on ${platform}?)"
        exit 1
    fi

    info "Unpacking"
    tar -xzf "${tmp_dir}/turbotokens.tar.gz" -C "$tmp_dir"

    mkdir -p "$install_dir"
    if ! mv "${tmp_dir}/turbotokens" "${install_dir}/turbotokens" 2>/dev/null; then
        info "No write permission to ${install_dir}; trying with sudo"
        sudo mv "${tmp_dir}/turbotokens" "${install_dir}/turbotokens"
    fi
    chmod +x "${install_dir}/turbotokens" 2>/dev/null || sudo chmod +x "${install_dir}/turbotokens"

    case ":${PATH}:" in
        *":${install_dir}:"*) ;;
        *)
            info "note: ${install_dir} is not on your PATH; add it with:"
            info "  export PATH=\"${install_dir}:\$PATH\""
            ;;
    esac

    version_out="$("${install_dir}/turbotokens" --version)"
    info "Installed turbotokens to ${install_dir}/turbotokens"
    info "Success: ${version_out}"
}

# Allow tests to source this file without running the installer.
if [ "${TURBOTOKENS_INSTALL_SOURCE_ONLY:-}" != "1" ]; then
    main "$@"
fi
