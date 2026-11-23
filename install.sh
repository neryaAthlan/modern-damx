#!/usr/bin/env bash
# Installs Modern DAMX system-wide. Run as root: sudo ./install.sh
#
#   sudo ./install.sh                     GUI + daemon, prebuilt from the latest release
#   sudo ./install.sh --build             build from source, then install
#   sudo ./install.sh --daemon-only       daemon + CLI, no GUI
#   sudo ./install.sh --with-driver       also install the Linuwu Sense driver
#   sudo ./install.sh --uninstall         remove what this script installed
set -euo pipefail

PREFIX="/usr"
LIBDIR="${PREFIX}/lib"
BINDIR="${PREFIX}/bin"
APPDIR="${PREFIX}/share/applications"
ICONDIR="${PREFIX}/share/icons/hicolor/scalable/apps"
UNITDIR="${LIBDIR}/systemd/system"
USERUNITDIR="${LIBDIR}/systemd/user"
GROUP="linuwu_sense"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

MODE="full"
SOURCE="prebuilt"
RELEASE_BASE="https://github.com/neryaAthlan/modern-damx/releases"
WITH_DRIVER=0
UNINSTALL=0
DRIVER_DIR="${DAMX_DRIVER_DIR:-}"

info() { printf '==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

usage() {
    cat <<EOF
Modern DAMX installer

Usage: sudo ./install.sh [options]

Options:
  --prebuilt            Download the binaries from the latest GitHub release
                        (default; no Rust toolchain needed)
  --build, --from-source
                        Compile Modern DAMX here, then install that build
  --daemon-only         Install only damxd and the damx CLI (no GUI)
  --with-driver         Also build and install the Linuwu Sense kernel module
  --driver-dir <path>   Driver checkout to use with --with-driver
                        (default: \$DAMX_DRIVER_DIR, then ../Div-Linuwu-Sense)
  --uninstall           Remove Modern DAMX (the boot preset is kept)
  -h, --help            Show this help

The default install downloads the prebuilt release for your architecture and
adds the GUI, the CLI, damxd, the desktop entry, and the daily update-check
timer. Pass --build to compile from source instead. The kernel module is never
replaced unless you pass --with-driver.
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --prebuilt) SOURCE="prebuilt" ;;
        --build|--from-source) SOURCE="build" ;;
        --daemon-only|--daemon) MODE="daemon" ;;
        --with-driver) WITH_DRIVER=1 ;;
        --driver-dir)
            shift
            [ $# -gt 0 ] || die "--driver-dir needs a path"
            DRIVER_DIR="$1"
            ;;
        --uninstall) UNINSTALL=1 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown option '$1' (try --help)" ;;
    esac
    shift
done

if [ "$(id -u)" -ne 0 ]; then
    die "run this installer as root: sudo ./install.sh"
fi

if [ "$UNINSTALL" -eq 1 ]; then
    info "Stopping damxd"
    systemctl disable --now damxd.service 2>/dev/null || true
    rm -f "${BINDIR}/damx" "${BINDIR}/damxd" "${BINDIR}/damx-gui"
    rm -f "${UNITDIR}/damxd.service" \
          "${USERUNITDIR}/damx-update-check.service" \
          "${USERUNITDIR}/damx-update-check.timer" \
          "${APPDIR}/io.github.neryaathlan.Damx.desktop" \
          "${ICONDIR}/io.github.neryaathlan.Damx.svg"
    systemctl daemon-reload || true
    info "Uninstalled. The boot preset in /etc/damxd/boot.toml and the kernel module were kept."
    exit 0
fi

DOWNLOAD_DIR=""
cleanup() {
    if [ -n "$DOWNLOAD_DIR" ]; then
        rm -rf "$DOWNLOAD_DIR"
    fi
}
trap cleanup EXIT

if [ "$SOURCE" = "prebuilt" ] && [ -x "${SCRIPT_DIR}/damxd" ] && [ -x "${SCRIPT_DIR}/damx" ]; then
    # Running from an extracted release tarball: use the binaries next to us.
    info "Using the prebuilt binaries in ${SCRIPT_DIR}"
    BIN_SRC="${SCRIPT_DIR}"
    DATA_DIR="${SCRIPT_DIR}/data"
else
    case "$SOURCE" in
        prebuilt)
            case "$(uname -m)" in
                x86_64|amd64) ARCH="x86_64" ;;
                aarch64|arm64) ARCH="aarch64" ;;
                *) die "no prebuilt binaries for $(uname -m); rerun with --build to compile from source" ;;
            esac
            command -v curl >/dev/null || die "curl is required to download the prebuilt binaries"
            command -v sha256sum >/dev/null || die "sha256sum is required to verify the download"

            info "Looking up the latest release"
            TAG_URL="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "${RELEASE_BASE}/latest")" \
                || die "could not reach GitHub; rerun with --build to compile from source"
            TAG="${TAG_URL##*/}"
            [ -n "$TAG" ] || die "could not determine the latest release tag"
            PKG="modern-damx-${TAG}-${ARCH}-linux.tar.gz"
            DOWNLOAD_DIR="$(mktemp -d)"

            info "Downloading ${PKG} from release ${TAG}"
            curl -fsSL --retry 3 -o "${DOWNLOAD_DIR}/${PKG}" \
                "${RELEASE_BASE}/download/${TAG}/${PKG}" \
                || die "no prebuilt ${ARCH} package in ${TAG}; rerun with --build to compile from source"
            curl -fsSL --retry 3 -o "${DOWNLOAD_DIR}/SHA256SUMS" \
                "${RELEASE_BASE}/download/${TAG}/SHA256SUMS" \
                || die "could not download SHA256SUMS for ${TAG}"
            (cd "$DOWNLOAD_DIR" && grep -F " ${PKG}" SHA256SUMS | sha256sum -c -) \
                || die "checksum verification failed for ${PKG}"

            tar -xzf "${DOWNLOAD_DIR}/${PKG}" -C "$DOWNLOAD_DIR"
            BIN_SRC="${DOWNLOAD_DIR}/modern-damx-${TAG}-${ARCH}-linux"
            DATA_DIR="${BIN_SRC}/data"
            ;;
        build)
            command -v cargo >/dev/null || die "cargo is required (install rustc and cargo)"
            if [ "$MODE" = "full" ]; then
                pkg-config --exists gtk4 || die "libgtk-4-dev is required for the GUI"
                pkg-config --exists libadwaita-1 || die "libadwaita-1-dev is required for the GUI"
            fi

            info "Building release binaries"
            if [ "$MODE" = "full" ]; then
                (cd "$SCRIPT_DIR" && cargo build --release --workspace)
            else
                (cd "$SCRIPT_DIR" && cargo build --release -p damx-daemon -p damx-cli)
            fi
            # We run as root, so give the build tree back to the user who invoked us.
            if [ -n "${SUDO_USER:-}" ] && [ -d "${SCRIPT_DIR}/target" ]; then
                chown -R "${SUDO_USER}:" "${SCRIPT_DIR}/target" 2>/dev/null || true
            fi
            BIN_SRC="${SCRIPT_DIR}/target/release"
            DATA_DIR="${SCRIPT_DIR}/data"
            ;;
    esac
fi

[ -x "${BIN_SRC}/damxd" ] && [ -x "${BIN_SRC}/damx" ] \
    || die "the package is missing the damxd and damx binaries"
if [ "$MODE" = "full" ]; then
    [ -x "${BIN_SRC}/damx-gui" ] || die "this package has no GUI binary; use --daemon-only"
fi
[ -f "${DATA_DIR}/damxd.service" ] || die "the package is missing its systemd units"

info "Installing binaries to ${BINDIR}"
install -d "${BINDIR}"
install -m 0755 "${BIN_SRC}/damxd" "${BINDIR}/damxd"
install -m 0755 "${BIN_SRC}/damx" "${BINDIR}/damx"
if [ "$MODE" = "full" ]; then
    install -m 0755 "${BIN_SRC}/damx-gui" "${BINDIR}/damx-gui"
fi

if ! getent group "${GROUP}" >/dev/null; then
    info "Creating group ${GROUP}"
    groupadd --system "${GROUP}"
fi

info "Installing systemd units"
install -d "${UNITDIR}" "${USERUNITDIR}"
install -m 0644 "${DATA_DIR}/damxd.service" "${UNITDIR}/damxd.service"
install -m 0644 "${DATA_DIR}/damx-update-check.service" "${USERUNITDIR}/damx-update-check.service"
install -m 0644 "${DATA_DIR}/damx-update-check.timer" "${USERUNITDIR}/damx-update-check.timer"

if [ "$MODE" = "full" ]; then
    info "Installing desktop entry and icon"
    install -d "${APPDIR}" "${ICONDIR}"
    install -m 0644 "${DATA_DIR}/io.github.neryaathlan.Damx.desktop" "${APPDIR}/io.github.neryaathlan.Damx.desktop"
    install -m 0644 "${DATA_DIR}/io.github.neryaathlan.Damx.svg" "${ICONDIR}/io.github.neryaathlan.Damx.svg"
    if command -v update-desktop-database >/dev/null; then
        update-desktop-database "${APPDIR}" || true
    fi
    if command -v gtk-update-icon-cache >/dev/null; then
        gtk-update-icon-cache -f -t "${PREFIX}/share/icons/hicolor" >/dev/null 2>&1 || true
    fi
fi

if [ "$WITH_DRIVER" -eq 1 ]; then
    if [ -z "$DRIVER_DIR" ]; then
        for candidate in "${SCRIPT_DIR}/../Div-Linuwu-Sense" "${SCRIPT_DIR}/../Div-Linuwu-Sense-main" "${SCRIPT_DIR}/../linuwu-sense"; do
            if [ -f "${candidate}/linuwu-sense.sh" ]; then
                DRIVER_DIR="$candidate"
                break
            fi
        done
    fi
    [ -n "$DRIVER_DIR" ] || die "no driver checkout found; pass --driver-dir /path/to/Div-Linuwu-Sense"
    [ -f "${DRIVER_DIR}/linuwu-sense.sh" ] || die "${DRIVER_DIR} does not contain linuwu-sense.sh"
    info "Installing the Linuwu Sense module from ${DRIVER_DIR}"
    bash "${DRIVER_DIR}/linuwu-sense.sh" install
fi

systemctl daemon-reload
info "Starting damxd"
systemctl enable --now damxd.service
# restart, not just start: an already running daemon must pick up the new binary
systemctl restart damxd.service

TARGET_USER="${SUDO_USER:-${USER:-}}"
if [ -n "${TARGET_USER}" ] && [ "${TARGET_USER}" != "root" ]; then
    USER_ID="$(id -u "${TARGET_USER}")"
    if [ -d "/run/user/${USER_ID}" ]; then
        info "Enabling the daily update check for ${TARGET_USER}"
        runuser -u "${TARGET_USER}" -- env XDG_RUNTIME_DIR="/run/user/${USER_ID}" \
            systemctl --user daemon-reload || true
        runuser -u "${TARGET_USER}" -- env XDG_RUNTIME_DIR="/run/user/${USER_ID}" \
            systemctl --user enable --now damx-update-check.timer || true
    fi
    if ! id -nG "${TARGET_USER}" | tr ' ' '\n' | grep -qx "${GROUP}"; then
        info "Adding ${TARGET_USER} to the ${GROUP} group (log out and back in to apply)"
        usermod -aG "${GROUP}" "${TARGET_USER}"
    fi
fi

if [ "$MODE" = "full" ]; then
    info "Done. Launch Modern DAMX from the application menu, or run 'damx status'."
else
    info "Done. damxd is running; control it with the damx CLI."
fi
