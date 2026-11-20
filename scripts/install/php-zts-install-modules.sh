#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "$SCRIPT_DIR/build-profile.sh"
php_zts_profile_init
php_zts_compiler_flags

# PIE installs extensions for the PHP ZTS runtime `php-zts-install.sh` provisions, on
# Linux, FreeBSD and macOS: the runtime is root-owned under /opt/php-zts-embed
# and services load it read-only. PHP_OWNER, PHP_GROUP and PHP_INSTALL_DIR
# override the defaults; macOS and FreeBSD name group 0 wheel.
PHP_OWNER="${PHP_OWNER:-root}"
case "$(uname -s)" in
Darwin | FreeBSD) PHP_GROUP="${PHP_GROUP:-wheel}" ;;
*) PHP_GROUP="${PHP_GROUP:-root}" ;;
esac

SUDO_READY=0
PIE_READY=0

PHP="$PHP_DIR/bin/php"
PHPIZE="$PHP_DIR/bin/phpize"
PHP_CONFIG="$PHP_DIR/bin/php-config"
PIE="${PIE:-}"
PHP_INI="${PHP_INI:-$PHP_DIR/etc/php.ini}"
PHP_SCAN_ROOT="${PHP_SCAN_ROOT:-$PHP_DIR/etc/conf.d}"
PHP_AVAILABLE_SCAN_DIR="${PHP_AVAILABLE_SCAN_DIR:-$PHP_SCAN_ROOT/available}"
PHP_EMBED_SCAN_DIR="${PHP_EMBED_SCAN_DIR:-$PHP_SCAN_ROOT/embed}"
PHP_CLI_SCAN_DIR="${PHP_CLI_SCAN_DIR:-$PHP_SCAN_ROOT/cli}"
#PHP_SCAN_DIR="$PHP_EMBED_SCAN_DIR"
PHP_EXT_DIR="${PHP_EXT_DIR:-}"
SKIP_APT="${SKIP_APT:-0}"
# Extensions this run was asked for that did not end up loaded.
NOT_LOADED=()

PATH="$PHP_DIR/bin:$PATH"
export PHPRC="$PHP_INI"
export PHP_INI_SCAN_DIR="$PHP_EMBED_SCAN_DIR"

BUILD_COMMANDS=(
  gcc
  autoconf
  pkg-config
)

# Request runtime extensions. These are enabled for both the embedded SAPI
# and CLI because PHP requests may use them directly under Claviron.
EXTENSIONS=(
  redis
)

# Development tooling for the standalone PHP CLI. Claviron tasks, daemons,
# applications, and request scripts all execute through libphp's embed SAPI;
# runtime capabilities therefore do not belong in this list.
#protobuf removed - 19.09.2026
#grpc removed - 19.09.2026
CLI_ONLY_EXTENSIONS=()
# sync not 8.5 yet

# Installed only by its explicit command, never by install or install-optional.
MANUAL_CLI_ONLY_EXTENSIONS=(
  xdebug
)

# Explicitly opted-in standalone CLI capabilities. Claviron does not depend on
# these for async execution: scheduling, wake-up, admission, and fleet
# coordination remain owned by Rust and Orbit.
OPTIONAL_CLI_ONLY_EXTENSIONS=(
  event
  uv
  igbinary
  msgpack
  zstd
  parallel
)

DEBIAN_PACKAGES=(
  build-essential
  autoconf
  automake
  libtool
  pkg-config
  re2c
  libpcre2-dev
  libzstd-dev
  libevent-dev
  libuv1-dev
)

ALMALINUX_PACKAGES=(
  gcc
  gcc-c++
  make
  autoconf
  automake
  libtool
  pkgconf-pkg-config
  re2c
  pcre2-devel
  libzstd-devel
  libevent-devel
  libuv-devel
)

FREEBSD_PACKAGES=(
  gmake
  autoconf
  automake
  libtool
  pkgconf
  re2c
  pcre2
  zstd
  libevent
  libuv
)

# macOS takes the extensions' build dependencies from Homebrew.
HOMEBREW_PACKAGES=(
  autoconf
  automake
  libtool
  pkg-config
  re2c
  pcre2
  zstd
  libevent
  libuv
  openssl@3
)

log() { printf '%s\n' "INFO: $*"; }
ok() { printf '%s\n' "OK: $*"; }
warn() { printf '%s\n' "WARN: $*" >&2; }
die() {
  printf '%s\n' "ERROR: $*" >&2
  exit 1
}

if [[ -t 1 ]]; then
  GREEN=$'\033[32m'
  RED=$'\033[31m'
  YELLOW=$'\033[33m'
  RESET=$'\033[0m'
else
  GREEN=""
  RED=""
  YELLOW=""
  RESET=""
fi

ensure_sudo() {
  if [[ "$SUDO_READY" == "1" ]]; then
    return 0
  fi

  command -v sudo >/dev/null 2>&1 || die "this step needs root privileges; run as root or as a user with sudo"
  # A passwordless sudo rule needs no validation; `sudo -v` would still ask for
  # a password when another rule for the same user requires one.
  if ! sudo -n true 2>/dev/null; then
    sudo -v || die "sudo authentication failed; run as root or use a user with sudo"
  fi
  SUDO_READY=1
}

root_cmd() {
  if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
    "$@"
  else
    ensure_sudo
    sudo "$@"
  fi
}

detect_os() {
  case "$(uname -s)" in
  Linux) echo linux ;;
  FreeBSD) echo freebsd ;;
  Darwin) echo macos ;;
  *) die "unsupported OS: $(uname -s)" ;;
  esac
}

detect_linux_family() {
  if [[ -r /etc/os-release ]]; then
    . /etc/os-release
    case " ${ID:-} ${ID_LIKE:-} " in
    *" debian "* | *" ubuntu "*) echo debian ;;
    *" rhel "* | *" fedora "* | *" centos "* | *" almalinux "* | *" rocky "*) echo rhel ;;
    *) echo unknown ;;
    esac
  else
    echo unknown
  fi
}

require_executable() {
  local path="$1"
  local label="$2"
  [[ -x "$path" ]] || die "$label not found or not executable: $path"
}

scan_dir_for_mode() {
  case "${1:-embed}" in
  embed) echo "$PHP_EMBED_SCAN_DIR" ;;
  cli) echo "$PHP_CLI_SCAN_DIR" ;;
  *) die "unknown scan mode: $1" ;;
  esac
}

extension_profile_version() {
  local ext="$1"
  local mode="$2"
  local scan_dir embed_version cli_version

  if [[ "$mode" == both ]]; then
    embed_version="$(extension_profile_version "$ext" embed)" || return 1
    cli_version="$(extension_profile_version "$ext" cli)" || return 1
    if [[ "$embed_version" == "$cli_version" ]]; then
      printf '%s\n' "$embed_version"
    else
      printf 'embed %s, cli %s\n' "$embed_version" "$cli_version"
    fi
    return
  fi

  scan_dir="$(scan_dir_for_mode "$mode")"
  PHP_INI_SCAN_DIR="$scan_dir" read_extension_version "$ext"
}

extension_so_exists() {
  local ext="$1"

  [[ -f "$PHP_EXT_DIR/${ext}.so" ]]
}

resolve_php_ext_dir() {
  if [[ -z "$PHP_EXT_DIR" ]]; then
    PHP_EXT_DIR="$("$PHP_CONFIG" --extension-dir)"
  fi
}

php_no_extensions() {
  PHPRC="$PHP_INI" PHP_INI_SCAN_DIR='' "$PHP" -n "$@"
}

check_php_build_contract() {
  local php_vernum
  local config_vernum
  local zts
  local sapis

  php_vernum="$(php_no_extensions -r 'echo PHP_VERSION_ID;')"
  config_vernum="$("$PHP_CONFIG" --vernum)"
  zts="$(php_no_extensions -r 'echo (defined("PHP_ZTS") && PHP_ZTS) ? "1" : "0";')"
  sapis="$("$PHP_CONFIG" --php-sapis)"

  [[ "$php_vernum" == "$config_vernum" ]] || die "php/php-config version mismatch: php=$php_vernum php-config=$config_vernum"
  [[ "$zts" == "1" ]] || die "PHP is not a ZTS build: $PHP"

  case " $sapis " in
  *" cli "* | *" cli"*)
    ;;
  *) die "PHP build is missing cli SAPI: $sapis" ;;
  esac

  case " $sapis " in
  *" embed "* | *" embed"*)
    ;;
  *) die "PHP build is missing embed SAPI: $sapis" ;;
  esac
}

# PHP starts without complaint when it cannot read php.ini and runs on its
# built-in defaults, so the file being present proves nothing: ask PHP.
require_loaded_ini() {
  local loaded

  [[ -f "$PHP_INI" ]] || die "php.ini is missing: $PHP_INI"
  [[ -n "$(find "$PHP_INI" -perm -0004 2>/dev/null)" ]] ||
    die "php.ini is not readable by other users: $PHP_INI (expected mode 0644; fix with: sudo chmod 0644 $PHP_INI)"
  loaded="$(PHPRC="$PHP_INI" PHP_INI_SCAN_DIR="$PHP_EMBED_SCAN_DIR" "$PHP" -r 'echo (string) php_ini_loaded_file();')"
  [[ "$loaded" == "$PHP_INI" ]] ||
    die "PHP did not load $PHP_INI as $(id -un) (loaded: ${loaded:-none})"
}

check_environment() {
  require_executable "$PHP" "php"
  require_executable "$PHP_CONFIG" "php-config"
  check_php_build_contract
  php_zts_require_profile
  require_loaded_ini
  resolve_php_ext_dir
  [[ "$PHP_EXT_DIR" == /* ]] || die "expected an absolute extension directory: $PHP_EXT_DIR"
}

prepare_pie() {
  [[ "$PIE_READY" == 0 ]] || return 0

  require_stable_pie
  require_executable "$PHPIZE" "phpize"
  install_os_deps
  ensure_extension_dir

  local cmd
  for cmd in "${BUILD_COMMANDS[@]}"; do
    command -v "$cmd" >/dev/null 2>&1 || die "missing command: $cmd"
  done

  if ! command -v make >/dev/null 2>&1 && ! command -v gmake >/dev/null 2>&1; then
    die "missing command: make or gmake"
  fi

  PIE_READY=1
}

require_stable_pie() {
  local version major minor

  if [[ -z "$PIE" ]]; then
    PIE="$(command -v pie)" || die "install a stable PHP Installer for Extensions (PIE) from https://github.com/php/pie/releases"
  fi
  require_executable "$PIE" "PIE"
  version="$("$PIE" --no-ansi --version)" || die "cannot run PIE at $PIE"
  if [[ ! "$version" =~ \(PIE\)[[:space:]]+([0-9]+)\.([0-9]+)\.[0-9]+$ ]]; then
    die "expected a stable PIE release at $PIE (reported: $version)"
  fi
  major="${BASH_REMATCH[1]}"
  minor="${BASH_REMATCH[2]}"
  if (( major < 1 || (major == 1 && minor < 5) )); then
    die "PIE 1.5.0 or newer is required (reported: $version)"
  fi
}

ensure_extension_dir() {
  if [[ -d "$PHP_EXT_DIR" ]]; then
    return 0
  fi

  root_cmd install -d -o "$PHP_OWNER" -g "$PHP_GROUP" -m 0755 "$PHP_EXT_DIR"
}

configure_freebsd_ca() {
  local system_ca="/etc/ssl/cert.pem"
  local private_ca="${PHP_PRIVATE_CA:-}"
  local ca_bundle="$PHP_DIR/etc/ca-bundle.pem"
  local ca_file="$system_ca"
  local ca_ini="$PHP_AVAILABLE_SCAN_DIR/10-ca.ini"

  [[ "$(detect_os)" == "freebsd" ]] || return 0
  [[ -r "$system_ca" ]] || die "FreeBSD CA bundle not found: $system_ca"

  root_cmd mkdir -p "$PHP_AVAILABLE_SCAN_DIR" "$PHP_EMBED_SCAN_DIR" "$PHP_CLI_SCAN_DIR"
  if [[ -n "$private_ca" ]]; then
    [[ -r "$private_ca" ]] || die "PHP_PRIVATE_CA is not readable: $private_ca"
    log "adding the operator-supplied authority to the PHP CA bundle"
    cat "$system_ca" "$private_ca" | root_cmd tee "$ca_bundle" >/dev/null
    ca_file="$ca_bundle"
  fi

  printf 'openssl.cafile=%s\n' "$ca_file" | root_cmd tee "$ca_ini" >/dev/null
  link_ini "$ca_ini" "$PHP_EMBED_SCAN_DIR"
  link_ini "$ca_ini" "$PHP_CLI_SCAN_DIR"
}

reset_scan_dirs() {
  root_cmd mkdir -p "$PHP_AVAILABLE_SCAN_DIR" "$PHP_EMBED_SCAN_DIR" "$PHP_CLI_SCAN_DIR"
}

extension_is_listed() {
  local wanted="$1"
  shift
  local ext

  for ext in "$@"; do
    [[ "$ext" == "$wanted" ]] && return 0
  done
  return 1
}

reconcile_scan_dir() {
  local scan_dir="$1"
  shift
  local ini_path
  local ini_name
  local ext

  while IFS= read -r -d '' ini_path; do
    ini_name="$(basename "$ini_path")"
    ext="${ini_name#50-}"
    ext="${ext%.ini}"
    if ! extension_is_listed "$ext" "$@"; then
      unlink_ini "$PHP_AVAILABLE_SCAN_DIR/$ini_name" "$scan_dir"
    fi
  done < <(find "$scan_dir" -maxdepth 1 -type l -name '50-*.ini' -print0)
}

reconcile_scan_profiles() {
  reset_scan_dirs
  reconcile_scan_dir "$PHP_EMBED_SCAN_DIR" "${EXTENSIONS[@]}"
  reconcile_scan_dir "$PHP_CLI_SCAN_DIR" \
    "${EXTENSIONS[@]}" \
    ${CLI_ONLY_EXTENSIONS[@]+"${CLI_ONLY_EXTENSIONS[@]}"} \
    "${OPTIONAL_CLI_ONLY_EXTENSIONS[@]}" \
    "${MANUAL_CLI_ONLY_EXTENSIONS[@]}"
}

link_ini() {
  local ini_path="$1"
  local scan_dir="$2"
  local ini_name

  ini_name="$(basename "$ini_path")"
  root_cmd mkdir -p "$scan_dir"
  local link="$scan_dir/$ini_name"
  if [[ ! -e "$link" && ! -L "$link" ]]; then
    root_cmd ln -s "$(ini_link_target "$ini_path" "$scan_dir")" "$link"
  fi
}

unlink_ini() {
  local ini_path="$1"
  local scan_dir="$2"
  local ini_name

  ini_name="$(basename "$ini_path")"
  local link="$scan_dir/$ini_name"
  if [[ -L "$link" && "$(readlink "$link")" == "$(ini_link_target "$ini_path" "$scan_dir")" ]]; then
    root_cmd rm -f "$link"
  fi
}

ini_link_target() {
  local ini_path="$1"
  local scan_dir="$2"
  if [[ "$(dirname "$scan_dir")" == "$(dirname "$PHP_AVAILABLE_SCAN_DIR")" && "$(basename "$PHP_AVAILABLE_SCAN_DIR")" == available ]]; then
    printf '../available/%s\n' "$(basename "$ini_path")"
  else
    printf '%s\n' "$ini_path"
  fi
}

enable_extension() {
  local module_name="$1"
  local so_path="$PHP_EXT_DIR/${module_name}.so"
  local mode="${2:-both}"
  local ini_path="$PHP_AVAILABLE_SCAN_DIR/50-${module_name}.ini"

  [[ -f "$so_path" ]] || return 0

  reset_scan_dirs
  if [[ ! -e "$ini_path" && ! -L "$ini_path" ]]; then
    printf '%s=%s.so\n' "$(extension_directive "$module_name")" "$module_name" |
      root_cmd tee "$ini_path" >/dev/null
    root_cmd chown "$PHP_OWNER:$PHP_GROUP" "$ini_path"
    root_cmd chmod 0644 "$ini_path"
  fi

  case "$mode" in
  both)
    link_ini "$ini_path" "$PHP_EMBED_SCAN_DIR"
    link_ini "$ini_path" "$PHP_CLI_SCAN_DIR"
    ;;
  cli)
    unlink_ini "$ini_path" "$PHP_EMBED_SCAN_DIR"
    link_ini "$ini_path" "$PHP_CLI_SCAN_DIR"
    ;;
  embed)
    link_ini "$ini_path" "$PHP_EMBED_SCAN_DIR"
    unlink_ini "$ini_path" "$PHP_CLI_SCAN_DIR"
    ;;
  *) die "unknown extension enable mode: $mode" ;;
  esac
}

print_debian_deps() {
  cat <<EOF
Install Debian/Ubuntu build dependencies first if needed:
  apt-get update
  apt-get install -y ${DEBIAN_PACKAGES[*]}
EOF
}

print_almalinux_deps() {
  cat <<EOF
Install AlmaLinux/RHEL build dependencies first if needed:
  dnf install -y ${ALMALINUX_PACKAGES[*]}
EOF
}

print_freebsd_deps() {
  cat <<EOF
Install FreeBSD build dependencies first if needed:
  pkg install -y ${FREEBSD_PACKAGES[*]}
EOF
}

print_macos_deps() {
  cat <<EOF
macOS extension build dependencies (Homebrew):
  brew install ${HOMEBREW_PACKAGES[*]}
EOF
}

install_debian_deps() {
  if [[ "$SKIP_APT" == "1" ]]; then
    log "SKIP_APT=1, skipping apt dependency install"
    return 0
  fi

  command -v apt-get >/dev/null 2>&1 || die "apt-get not found; install the build dependencies yourself and set SKIP_APT=1"

  log "installing Debian extension build dependencies"
  root_cmd apt-get update
  root_cmd apt-get install -y "${DEBIAN_PACKAGES[@]}"
}

install_almalinux_deps() {
  if [[ "$SKIP_APT" == "1" ]]; then
    log "SKIP_APT=1, skipping package dependency install"
    return 0
  fi

  command -v dnf >/dev/null 2>&1 || command -v yum >/dev/null 2>&1 || die "dnf/yum not found; install the build dependencies yourself and set SKIP_APT=1"

  local installer="dnf"
  command -v dnf >/dev/null 2>&1 || installer="yum"

  log "installing AlmaLinux/RHEL extension build dependencies"
  root_cmd "$installer" install -y "${ALMALINUX_PACKAGES[@]}"
}

install_freebsd_deps() {
  if [[ "$SKIP_APT" == "1" ]]; then
    log "SKIP_APT=1, skipping package dependency install"
    return 0
  fi

  command -v pkg >/dev/null 2>&1 || die "pkg not found; install the build dependencies yourself and set SKIP_APT=1"

  log "installing FreeBSD extension build dependencies"
  root_cmd pkg install -y "${FREEBSD_PACKAGES[@]}"
}

install_macos_deps() {
  if [[ "$SKIP_APT" == "1" ]]; then
    log "SKIP_APT=1, skipping Homebrew dependency install"
  else
    command -v brew >/dev/null 2>&1 || die "Homebrew not found; install it from https://brew.sh"
    log "installing macOS extension build dependencies (Homebrew)"
    brew install "${HOMEBREW_PACKAGES[@]}"
  fi

  # Several of these are keg-only; put their prefixes on the paths the PIE builds receive.
  command -v brew >/dev/null 2>&1 || die "Homebrew not found; install it from https://brew.sh"
  local package prefix
  for package in pcre2 zstd libevent libuv openssl@3; do
    prefix="$(brew --prefix "$package")"
    CPPFLAGS="-I$prefix/include${CPPFLAGS:+ $CPPFLAGS}"
    LDFLAGS="-L$prefix/lib${LDFLAGS:+ $LDFLAGS}"
    if [[ -d "$prefix/lib/pkgconfig" ]]; then
      PKG_CONFIG_PATH="$prefix/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
    fi
  done
  export CPPFLAGS LDFLAGS PKG_CONFIG_PATH
}

install_os_deps() {
  case "$(detect_os)" in
  linux)
    case "$(detect_linux_family)" in
    debian) install_debian_deps ;;
    rhel) install_almalinux_deps ;;
    *) [[ "$SKIP_APT" == "1" ]] || die "unknown Linux family; install the build dependencies yourself and set SKIP_APT=1" ;;
    esac
    ;;
  freebsd) install_freebsd_deps ;;
  macos) install_macos_deps ;;
  esac
}

extension_directive() {
  case "$1" in
  xdebug|opcache) echo zend_extension ;;
  *) echo extension ;;
  esac
}

read_extension_version() {
  local ext="$1"
  shift

  "$PHP" "$@" -d display_errors=stderr -r '
    $version = phpversion($argv[1]);
    if ($version === false) {
      exit(1);
    }
    echo $version;
  ' -- "$ext"
}

extension_file_version() {
  local ext="$1"
  local so_path="$2"

  PHP_INI_SCAN_DIR='' read_extension_version "$ext" -n -d "$(extension_directive "$ext")=$so_path"
}

is_stable_version() {
  [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
}

build_stable_extension() (
  local module_name="$1"
  local package="$2"
  shift 2
  local work_dir staged_so version target
  local installed_tmp=""

  work_dir="$(mktemp -d "${TMPDIR:-/tmp}/php-zts-pie.XXXXXXXX")" || exit 1
  trap 'rm -rf -- "$work_dir"; if [[ -n "$installed_tmp" ]]; then root_cmd rm -f -- "$installed_tmp"; fi' EXIT
  export TMPDIR="$work_dir" PIE_WORKING_DIRECTORY="$work_dir/pie"
  export INSTALL_ROOT="$work_dir/stage"
  staged_so="$INSTALL_ROOT$PHP_EXT_DIR/$module_name.so"
  mkdir -p -- "${staged_so%/*}" || exit 1

  # Compile as the invoking user and validate before replacing a live .so.
  "$PIE" --no-interaction --no-ansi install \
    --with-php-config="$PHP_CONFIG" --with-phpize-path="$PHPIZE" \
    --skip-enable-extension --no-cache \
    --suppress-download-url-method=pre-packaged-binary \
    "$package:*@stable" "$@" || exit 1

  if [[ ! -f "$staged_so" ]] || ! version="$(extension_file_version "$module_name" "$staged_so")"; then
    warn "PIE output did not load from $staged_so"
    exit 1
  fi
  is_stable_version "$version" || {
    warn "$module_name built version $version, expected a stable release"
    exit 1
  }

  target="$PHP_EXT_DIR/$module_name.so"
  installed_tmp="$(root_cmd mktemp "$target.XXXXXXXX")" || exit 1
  root_cmd install -o "$PHP_OWNER" -g "$PHP_GROUP" -m 0644 "$staged_so" "$installed_tmp" || exit 1
  root_cmd mv -f -- "$installed_tmp" "$target" || exit 1
  installed_tmp=""
  ok "installed with PIE: $module_name $version"
)

pie_install_one() {
  local module_name="$1"
  local mode="$2"
  local package version=""
  local configure_options=()

  if version="$(extension_profile_version "$module_name" "$mode")"; then
    log "using existing extension: $module_name $version ($mode)"
    return 0
  fi
  version=""
  if [[ "$mode" == both ]]; then
    # A module can require other active extensions and fail to load under -n.
    # Preserve it if either configured profile already loads it.
    version="$(extension_profile_version "$module_name" embed ||
      extension_profile_version "$module_name" cli)" || version=""
  fi
  if [[ -z "$version" ]] && extension_so_exists "$module_name"; then
    version="$(extension_file_version "$module_name" "$PHP_EXT_DIR/$module_name.so")" || version=""
  fi

  if [[ -n "$version" ]]; then
    log "using existing extension: $module_name $version"
  else
    case "$module_name" in
    redis) package=phpredis/phpredis ;;
    xdebug) package=xdebug/xdebug ;;
    event)
      package=osmanov/pecl-event
      configure_options+=(--with-event-pthreads)
      ;;
    igbinary) package=pie-extensions/igbinary ;;
    msgpack) package=msgpack/msgpack-php ;;
    zstd) package=kjdev/zstd ;;
    parallel) package=pecl/parallel ;;
    uv)
      warn "uv has no stable upstream release"
      NOT_LOADED+=("$module_name")
      return 0
      ;;
    *) die "no PIE package mapped for $module_name" ;;
    esac
    prepare_pie
    log "installing a stable release of $package with PIE"
    if ! build_stable_extension "$module_name" "$package" ${configure_options[@]+"${configure_options[@]}"}; then
      warn "could not install a stable compatible release of $module_name"
      NOT_LOADED+=("$module_name")
      return 0
    fi
  fi

  enable_extension "$module_name" "$mode"
  if version="$(extension_profile_version "$module_name" "$mode")"; then
    ok "loaded: $module_name $version ($mode)"
  else
    warn "not loaded: $module_name ($mode)"
    NOT_LOADED+=("$module_name")
  fi
}

install_optional_extensions() {
  local ext

  for ext in "${OPTIONAL_CLI_ONLY_EXTENSIONS[@]}"; do
    pie_install_one "$ext" cli
  done
}

# getent is not available on macOS; id and dscl answer the same question there.
account_exists() {
  local kind="$1" name="$2"
  if command -v getent >/dev/null 2>&1; then
    getent "$kind" "$name" >/dev/null 2>&1
  elif [[ "$kind" == passwd ]]; then
    id -u "$name" >/dev/null 2>&1
  else
    dscl . -read "/Groups/$name" >/dev/null 2>&1
  fi
}

# The run's verdict: every requested extension loaded, or a failure naming
# those that could not be loaded.
report_result() {
  if [[ "${#NOT_LOADED[@]}" -gt 0 ]]; then
    die "not loaded for PHP $("$PHP" -n -r 'echo PHP_VERSION;'): ${NOT_LOADED[*]}; see the build output above"
  fi
  ok "all requested extensions are loaded"
}

finish_runtime_install() {
  require_loaded_ini

  if [[ "$PHP_BUILD_PROFILE" == debug ]]; then
    log 'Debug extensions stay in the private debug prefix; ldconfig is unchanged.'
    return 0
  fi
  case "$(detect_os)" in
  freebsd)
    root_cmd ldconfig -m "$PHP_DIR/lib"
    ;;
  linux)
    printf '%s\n' "$PHP_DIR/lib" | root_cmd tee /etc/ld.so.conf.d/php-zts-embed.conf >/dev/null
    root_cmd ldconfig
    ;;
  macos)
    # No loader cache on macOS: PHP loads extensions by path from its
    # extension_dir, and libphp already carries its own install name.
    ;;
  esac
}

prepare_install() {
  account_exists passwd "$PHP_OWNER" && account_exists group "$PHP_GROUP" ||
    die "owner $PHP_OWNER:$PHP_GROUP does not exist; set PHP_OWNER and PHP_GROUP"
  check_environment
  configure_freebsd_ca
  reconcile_scan_profiles
}

install_all() {
  local ext

  prepare_install

  for ext in "${EXTENSIONS[@]}"; do
    pie_install_one "$ext" both
  done

  for ext in ${CLI_ONLY_EXTENSIONS[@]+"${CLI_ONLY_EXTENSIONS[@]}"}; do
    pie_install_one "$ext" cli
  done

  if [[ "${INSTALL_OPTIONAL:-0}" == "1" ]]; then
    install_optional_extensions
  else
    log "optional extensions skipped; run install-optional or set INSTALL_OPTIONAL=1"
  fi

  finish_runtime_install
  report_result
}

install_embed() {
  local ext

  prepare_install

  for ext in "${EXTENSIONS[@]}"; do
    pie_install_one "$ext" both
  done

  finish_runtime_install
  report_result
}

install_optional() {
  prepare_install
  install_optional_extensions
  finish_runtime_install
  report_result
}

install_xdebug() {
  prepare_install
  pie_install_one xdebug cli
  finish_runtime_install
  report_result
}

verify() {
  local ext version
  local result=0

  check_environment
  echo "php is $PHP"
  echo "php-config is $PHP_CONFIG"
  echo "extension dir is $PHP_EXT_DIR"
  echo "embed scan dir is $PHP_EMBED_SCAN_DIR"
  echo "cli scan dir is $PHP_CLI_SCAN_DIR"
  echo

  printf '%s\n' "Embed + CLI extensions:"
  for ext in "${EXTENSIONS[@]}"; do
    if version="$(extension_profile_version "$ext" both)"; then
      printf '%bOK%b      %-12s embed+cli  %s\n' "$GREEN" "$RESET" "$ext" "$version"
    else
      printf '%bMISSING%b %-12s must load in both embed and CLI profiles\n' "$RED" "$RESET" "$ext"
      result=1
    fi
  done

  echo
  printf '%s\n' "CLI-only extensions:"
  for ext in ${CLI_ONLY_EXTENSIONS[@]+"${CLI_ONLY_EXTENSIONS[@]}"} "${OPTIONAL_CLI_ONLY_EXTENSIONS[@]}" "${MANUAL_CLI_ONLY_EXTENSIONS[@]}"; do
    if version="$(extension_profile_version "$ext" embed)"; then
      printf '%bUNEXPECTED%b %-12s embed      %s\n' "$RED" "$RESET" "$ext" "$version"
      result=1
    elif version="$(extension_profile_version "$ext" cli)"; then
      printf '%bOK%b      %-12s cli        %s\n' "$GREEN" "$RESET" "$ext" "$version"
    elif extension_is_listed "$ext" ${CLI_ONLY_EXTENSIONS[@]+"${CLI_ONLY_EXTENSIONS[@]}"}; then
      printf '%bMISSING%b %-12s must load in the CLI profile\n' "$RED" "$RESET" "$ext"
      result=1
    else
      printf '%bSKIP%b    %-12s cli        opt-in\n' "$YELLOW" "$RESET" "$ext"
    fi
  done

  return "$result"
}

info() {
  check_environment
  "$PHP" -v
  echo "-------------------------"
  "$PHP_CONFIG" --php-sapis
  echo "-------------------------"
  "$PHP" --ini || true
}

usage() {
  cat <<'EOF'
Usage: scripts/install/php-zts-install-modules.sh [install|install-embed|install-xdebug|install-optional|verify|info|deps]

Commands:
  install           Enable required runtime extensions for embed and CLI
  install-embed     Enable runtime extensions without CLI-only tooling
  install-xdebug    Install and enable Xdebug for CLI only
  install-optional  Enable opt-in CLI extensions
  verify            Check which extensions load under the embed and CLI INI profiles
  info              Show PHP version, SAPIs, and ini paths
  deps              Print package dependencies for this OS

Environment:
  INSTALL_OPTIONAL=1  Also attempt optional CLI-only extensions during install
  PIE                Path to a stable PIE executable (default: pie on PATH)
  PHP_BUILD_PROFILE   release (default) or debug; must match the installed PHP
  PHP_INSTALL_DIR     Explicit installation prefix, including for debug modules

Existing loadable extensions are reused. Missing extensions require a stable PIE release.
Xdebug is separate from PHP_DEBUG and is installed only by install-xdebug.
EOF
}

main() {
  local cmd="${1:-help}"

  case "$cmd" in
  help|-h|--help) usage ;;
  install) install_all ;;
  install-embed) install_embed ;;
  install-xdebug) install_xdebug ;;
  install-optional) install_optional ;;
  verify) verify ;;
  info) info ;;
  deps)
    case "$(detect_os)" in
    linux)
      case "$(detect_linux_family)" in
      rhel) print_almalinux_deps ;;
      *) print_debian_deps ;;
      esac
      ;;
    freebsd) print_freebsd_deps ;;
    macos) print_macos_deps ;;
    esac
    ;;
  *)
    usage
    exit 1
    ;;
  esac
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then main "$@"; fi
