#!/usr/bin/env bash
set -euo pipefail

# Resolve companion files before source preparation changes the working directory.
PHP_INSTALL_SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "$PHP_INSTALL_SCRIPT_DIR/build-profile.sh"
php_zts_profile_init

PHP_VERSION="${PHP_VERSION:-8.5.13}"
RECONFIGURE=0
PHP_SOURCE_CHANGED=0

# This script provisions the ZTS PHP build on Linux, FreeBSD and macOS from the
# official php-src release tag: the developer builds, and the result is a
# root-owned runtime under /opt/php-zts-embed that services load read-only.
# The configure options are the same on every OS; only where the build
# dependencies come from differs (apt/dnf, pkg, Homebrew). PHP_OWNER,
# PHP_GROUP and PHP_INSTALL_DIR override the defaults.
#
# The build is for production, tuned for how Claviron uses it: a host links
# libphp at startup and runs ZTS requests in its own threads.
#   release (default)  -O2 -g0, PHP debug and debug assertions off. On ELF
#                      (Linux, FreeBSD) libphp also gets
#                      -fno-semantic-interposition, so its calls into itself
#                      are direct and inlinable instead of going through the
#                      PLT, and -ftls-model=initial-exec, so the per-request
#                      thread-local state ZTS reads constantly is one register
#                      offset instead of a __tls_get_addr call. Valid because
#                      hosts load libphp at startup, not with dlopen. macOS
#                      gets neither: Mach-O symbols are not interposable and
#                      its thread locals use one mechanism. PECL extensions
#                      never get them; they are loaded with dlopen.
#   debug              -O0 -g, PHP debug on, its own source tree and prefix.
# Not yet in release, each to be measured first: PGO (php-src's make
# prof-gen / prof-use with a training workload), and -march=native, which
# would tie the binary to the CPU it was built on and so stays opt-in.
#
# Usage:
#   scripts/install/php-zts-install.sh            # show help
#   scripts/install/php-zts-install.sh install    # build + install without cleaning
#   scripts/install/php-zts-install.sh clean      # clean the source build tree
#   scripts/install/php-zts-install.sh deps           # print package deps only
#   scripts/install/php-zts-install.sh dry-run        # show resolved paths
#   scripts/install/php-zts-install.sh check-source   # prepare php-src release tag only
#   scripts/install/php-zts-install.sh verify         # smoke-test installed php

# Who owns the installed runtime. Root by default: a service that loads
# libphp must not be able to rewrite it. macOS and FreeBSD name group 0 wheel.
PHP_OWNER="${PHP_OWNER:-root}"
case "$(uname -s)" in
Darwin | FreeBSD) PHP_GROUP="${PHP_GROUP:-wheel}" ;;
*) PHP_GROUP="${PHP_GROUP:-root}" ;;
esac
PHP_CONFIG_DIR="${PHP_CONFIG_DIR:-$PHP_DIR/etc}"
PHP_SCAN_ROOT="${PHP_SCAN_ROOT:-$PHP_CONFIG_DIR/conf.d}"
PHP_EMBED_SCAN_DIR="${PHP_EMBED_SCAN_DIR:-$PHP_SCAN_ROOT/embed}"
PHP_CLI_SCAN_DIR="${PHP_CLI_SCAN_DIR:-$PHP_SCAN_ROOT/cli}"
PHP_AVAILABLE_SCAN_DIR="${PHP_AVAILABLE_SCAN_DIR:-$PHP_SCAN_ROOT/available}"
PHP_SCAN_DIR="${PHP_SCAN_DIR:-$PHP_EMBED_SCAN_DIR}"
MAKE_JOBS="${MAKE_JOBS:-}"
PHP_ENABLE_FPM="${PHP_ENABLE_FPM:-1}"
SKIP_APT="${SKIP_APT:-0}"
SUDO_READY=0

DEBIAN_PACKAGES=(
  git
  build-essential
  autoconf
  automake
  bison
  re2c
  pkg-config
  libtool
  libxml2-dev
  libssl-dev
  libsqlite3-dev
  zlib1g-dev
  libcurl4-openssl-dev
  libbz2-dev
  libzip-dev
  libicu-dev
  libonig-dev
  libpng-dev
  libjpeg-dev
  libwebp-dev
  libfreetype-dev
  libpq-dev
  libreadline-dev
  libsodium-dev
  libargon2-dev
  libxslt1-dev
  libgmp-dev
)

ALMALINUX_PACKAGES=(
  git
  curl
  gcc
  gcc-c++
  clang
  clang-devel
  llvm-devel
  make
  autoconf
  automake
  bison
  pkgconf-pkg-config
  libtool
  libxml2-devel
  openssl-devel
  sqlite-devel
  zlib-devel
  libcurl-devel
  bzip2-devel
  libzip-devel
  libicu-devel
  oniguruma-devel
  libpng-devel
  libjpeg-turbo-devel
  libwebp-devel
  freetype-devel
  postgresql-devel
  readline-devel
  libsodium-devel
  libargon2-devel
  libxslt-devel
  gmp-devel
  xz
)

FREEBSD_PACKAGES=(
  git
  gmake
  autoconf
  automake
  bison
  re2c
  pkgconf
  libtool
  libiconv
  libxml2
  openssl
  sqlite3
  curl
  bzip2
  libzip
  icu
  oniguruma
  png
  jpeg-turbo
  webp
  freetype2
  postgresql18-client
  readline
  libsodium
  libargon2
  libxslt
  gmp
)

# macOS takes its build dependencies from Homebrew. Several are keg-only, so
# configure_macos_env puts each one's prefix on the build paths.
HOMEBREW_PACKAGES=(
  autoconf
  automake
  bison
  re2c
  pkg-config
  libtool
  openssl@3
  libxml2
  libxslt
  sqlite
  zlib
  bzip2
  curl
  libzip
  icu4c
  oniguruma
  libpng
  jpeg-turbo
  webp
  freetype
  libpq
  mysql-client
  libsodium
  argon2
  gmp
)

FAILED() { printf '❌ \033[1;31m[FAILED]\033[0m %s\n' "$*" >&2; }
WARNING() { printf '\033[1;33m[WARNING]\033[0m %s\n' "$*" >&2; }
INFO() { printf '\033[1;34m[INFO]\033[0m %s\n' "$*" >&2; }
SUCCESS() { printf '\033[1;32m[✔]\033[0m %s\n' "$*" >&2; }

log() { INFO "$*"; }
die() {
  FAILED "$*"
  exit 1
}

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

sudo_cmd() {
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

jobs_count() {
  if [[ -n "$MAKE_JOBS" ]]; then
    echo "$MAKE_JOBS"
  elif command -v nproc >/dev/null 2>&1; then
    nproc
  elif command -v sysctl >/dev/null 2>&1; then
    sysctl -n hw.ncpu
  else
    die "cannot count CPUs; set MAKE_JOBS"
  fi
}

version_ge() {
  awk -v version="$1" -v minimum="$2" '
    BEGIN {
      split(version, v, ".")
      split(minimum, m, ".")
      for (i = 1; i <= 3; i++) {
        vi = v[i] + 0
        mi = m[i] + 0
        if (vi > mi) exit 0
        if (vi < mi) exit 1
      }
      exit 0
    }
  '
}

re2c_version() {
  command -v re2c >/dev/null 2>&1 || return 1
  re2c --version 2>/dev/null | awk 'NR == 1 {print $2}'
}

# The packages a list names, minus those whose library pkg-config already
# finds: an operator who supplies a library through PKG_CONFIG_PATH keeps it,
# and no package manager installs a second copy beside it. Applies to every
# OS's list; before pkg-config itself is installed, nothing is dropped.
without_provided() {
  local package
  for package in "$@"; do
    case "$package" in
    libzip-dev | libzip-devel | libzip)
      if command -v pkg-config >/dev/null 2>&1 && pkg-config --exists libzip; then
        log "libzip found through pkg-config; not installing $package"
        continue
      fi
      ;;
    esac
    printf '%s\n' "$package"
  done
}

# Fill the named array with a package list minus what is already provided.
# A read loop, not mapfile: macOS ships bash 3.2.
select_packages() {
  local target="$1"
  shift
  local line
  eval "$target=()"
  while IFS= read -r line; do
    eval "$target+=(\"\$line\")"
  done < <(without_provided "$@")
}

print_debian_deps() {
  cat <<EOF
Debian/Ubuntu dependencies:
  apt-get update
  apt-get install -y ${DEBIAN_PACKAGES[*]}
EOF
}

print_almalinux_deps() {
  cat <<EOF
AlmaLinux/RHEL dependencies:
  dnf install -y ${ALMALINUX_PACKAGES[*]}
EOF
}

print_freebsd_deps() {
  cat <<EOF
FreeBSD dependencies:
  pkg install -y ${FREEBSD_PACKAGES[*]}
EOF
}

print_macos_deps() {
  cat <<EOF
macOS dependencies (Homebrew):
  brew install ${HOMEBREW_PACKAGES[*]}
EOF
}

install_debian_deps() {
  if [[ "$SKIP_APT" == "1" ]]; then
    log "SKIP_APT=1, skipping apt dependency install"
    return 0
  fi

  command -v apt-get >/dev/null 2>&1 || die "apt-get not found; install the build dependencies yourself and set SKIP_APT=1"

  local packages
  select_packages packages "${DEBIAN_PACKAGES[@]}"
  log "installing Debian build dependencies"
  sudo_cmd apt-get update
  sudo_cmd apt-get install -y "${packages[@]}"
}

install_almalinux_deps() {
  if [[ "$SKIP_APT" == "1" ]]; then
    log "SKIP_APT=1, skipping package dependency install"
    return 0
  fi

  command -v dnf >/dev/null 2>&1 || command -v yum >/dev/null 2>&1 || die "dnf/yum not found; install the build dependencies yourself and set SKIP_APT=1"

  local installer="dnf"
  command -v dnf >/dev/null 2>&1 || installer="yum"

  local packages
  select_packages packages "${ALMALINUX_PACKAGES[@]}"
  log "installing AlmaLinux/RHEL build dependencies"
  sudo_cmd "$installer" install -y "${packages[@]}"
}

install_freebsd_deps() {
  if [[ "$SKIP_APT" == "1" ]]; then
    log "SKIP_APT=1, skipping package dependency install"
    return 0
  fi

  command -v pkg >/dev/null 2>&1 || die "pkg not found; install the build dependencies yourself and set SKIP_APT=1"

  local packages
  select_packages packages "${FREEBSD_PACKAGES[@]}"
  log "installing FreeBSD build dependencies"
  sudo_cmd pkg install -y "${packages[@]}"
}

install_macos_deps() {
  if [[ "$SKIP_APT" == "1" ]]; then
    log "SKIP_APT=1, skipping Homebrew dependency install"
    return 0
  fi

  command -v brew >/dev/null 2>&1 || die "Homebrew not found; install it from https://brew.sh"

  local packages
  select_packages packages "${HOMEBREW_PACKAGES[@]}"
  log "installing macOS build dependencies (Homebrew)"
  brew install "${packages[@]}"
}

configure_linux_env() {
  case "$(detect_linux_family)" in
  debian) install_debian_deps ;;
  rhel) install_almalinux_deps ;;
  *) [[ "$SKIP_APT" == "1" ]] || die "unknown Linux family; install the build dependencies yourself and set SKIP_APT=1" ;;
  esac
  export PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-}"
}

configure_freebsd_env() {
  install_freebsd_deps
  MAKE="${MAKE:-gmake}"
  export MAKE

  local ssl_dir="/usr/local"
  CPPFLAGS="-I${ssl_dir}/include${CPPFLAGS:+ $CPPFLAGS}"
  LDFLAGS="-L${ssl_dir}/lib${LDFLAGS:+ $LDFLAGS}"
  PKG_CONFIG_PATH="/usr/local/libdata/pkgconfig:/usr/local/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
  export CPPFLAGS LDFLAGS PKG_CONFIG_PATH
}

# Put one Homebrew package's prefix on the build paths; keg-only packages are
# not on the default compiler or pkg-config paths.
add_homebrew_package() {
  local prefix
  prefix="$(brew --prefix "$1")"
  PATH="$prefix/bin:$PATH"
  CPPFLAGS="-I$prefix/include${CPPFLAGS:+ $CPPFLAGS}"
  LDFLAGS="-L$prefix/lib${LDFLAGS:+ $LDFLAGS}"
  if [[ -d "$prefix/lib/pkgconfig" ]]; then
    PKG_CONFIG_PATH="$prefix/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
  fi
}

configure_macos_env() {
  install_macos_deps
  command -v brew >/dev/null 2>&1 || die "Homebrew not found; install it from https://brew.sh"

  local package
  for package in bison re2c openssl@3 libxml2 libxslt sqlite zlib bzip2 curl \
    libzip icu4c oniguruma libpng jpeg-turbo webp freetype libpq mysql-client \
    libsodium argon2 gmp; do
    add_homebrew_package "$package"
  done

  # pkg-config alone misses some of ICU's libraries.
  local icu
  icu="$(brew --prefix icu4c)"
  ICU_CFLAGS="-I$icu/include${ICU_CFLAGS:+ $ICU_CFLAGS}"
  ICU_LIBS="-L$icu/lib -licui18n -licuio -licuuc -licudata${ICU_LIBS:+ $ICU_LIBS}"
  export PATH CPPFLAGS LDFLAGS PKG_CONFIG_PATH ICU_CFLAGS ICU_LIBS
}

prepare_source() {
  local release_tag="php-$PHP_VERSION"
  local local_tag_sha
  local remote_tag_sha

  mkdir -p "$(dirname "$PHP_SRC_DIR")"
  [[ -d "$(dirname "$PHP_DIR")" ]] || sudo_cmd mkdir -p "$(dirname "$PHP_DIR")"

  if [[ ! -d "$PHP_SRC_DIR/.git" ]]; then
    log "php-src not found, cloning upstream → $PHP_SRC_DIR"
    git clone https://github.com/php/php-src.git "$PHP_SRC_DIR"
  fi

  cd "$PHP_SRC_DIR"

  if ! git rev-parse --verify --quiet "refs/tags/$release_tag" >/dev/null; then
    git fetch --no-tags origin "refs/tags/$release_tag:refs/tags/$release_tag" ||
      die "PHP release tag $release_tag not found; unsupported PHP_VERSION=$PHP_VERSION"
  fi

  local_tag_sha="$(git rev-parse "refs/tags/$release_tag")"
  remote_tag_sha="$(git ls-remote --tags origin "refs/tags/$release_tag" | awk '{print $1}')"
  if [[ -z "$remote_tag_sha" ]]; then
    die "PHP release tag $release_tag not found on origin; unsupported PHP_VERSION=$PHP_VERSION"
  fi
  if [[ "$local_tag_sha" != "$remote_tag_sha" ]]; then
    die "local PHP release tag $release_tag differs from origin; delete the local tag and retry"
  fi

  if [[ "$(git rev-parse HEAD)" != "$(git rev-parse "refs/tags/$release_tag^{commit}")" ]]; then
    PHP_SOURCE_CHANGED=1
  fi
  log "switching php-src to release tag $release_tag"
  git switch --detach "refs/tags/$release_tag"
}

# Release flags for libphp on ELF (Linux, FreeBSD). libphp is a shared
# library: without -fno-semantic-interposition its calls into itself go
# through the PLT and are not inlined, and hosts load it at startup, so ZTS's
# thread-local state can use the initial-exec model instead of
# __tls_get_addr. Mach-O needs neither: its symbols are not interposable (two-
# level namespace) and its thread locals use one mechanism. Only libphp gets
# them; PECL extensions are loaded with dlopen, where initial-exec is unsafe.
php_zts_libphp_flags() {
  [[ "$PHP_BUILD_PROFILE" == release ]] || return 0
  case "$1" in
  linux | freebsd) printf '%s' '-fno-semantic-interposition -ftls-model=initial-exec' ;;
  esac
}

configure_build_env() {
  local os="$1"

  export PHP_DIR PHP_CONFIG_DIR PHP_SCAN_ROOT PHP_EMBED_SCAN_DIR PHP_CLI_SCAN_DIR PHP_AVAILABLE_SCAN_DIR PHP_SCAN_DIR
  export PHP="$PHP_DIR/bin/php"
  export PHPIZE="$PHP_DIR/bin/phpize"
  export PHP_CONFIG="$PHP_DIR/bin/php-config"
  export PATH="$PHP_DIR/bin:$PATH"

  case "$os" in
  linux)
    configure_linux_env
    ;;
  freebsd) configure_freebsd_env ;;
  macos) configure_macos_env ;;
  esac

  export LDFLAGS="${LDFLAGS:-}"
  export CPPFLAGS="${CPPFLAGS:-}"
  export PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-}"
  php_zts_compiler_flags
  local libphp_flags
  libphp_flags="$(php_zts_libphp_flags "$os")"
  if [[ -n "$libphp_flags" ]]; then
    CFLAGS="$CFLAGS $libphp_flags"
    CXXFLAGS="$CXXFLAGS $libphp_flags"
    export CFLAGS CXXFLAGS
  fi
}

verify_re2c() {
  local min_version="1.0.3"
  local found

  found="$(re2c_version || true)"
  if [[ -n "$found" ]] && version_ge "$found" "$min_version"; then
    log "re2c $found is available"
    return 0
  fi

  if [[ -n "$found" ]]; then
    printf 'ERROR: re2c %s is too old; PHP requires >= %s\n' "$found" "$min_version" >&2
  else
    printf 'ERROR: re2c is missing; PHP requires >= %s\n' "$min_version" >&2
  fi

  exit 1
}

verify_pkg_config() {
  local missing=()
  local packages=(
    libxml-2.0
    openssl
    sqlite3
    zlib
    libpng
    libjpeg
    libwebp
    freetype2
    libcurl
    libzip
    icu-i18n
    icu-io
    icu-uc
    libpq
  )

  for package in "${packages[@]}"; do
    if ! pkg-config --exists "$package"; then
      missing+=("$package")
    fi
  done

  if [[ "${#missing[@]}" -gt 0 ]]; then
    printf 'Missing pkg-config packages:\n' >&2
    printf ' - %s\n' "${missing[@]}" >&2
    case "$(detect_os)" in
    linux)
      case "$(detect_linux_family)" in
      rhel) print_almalinux_deps >&2 ;;
      *) print_debian_deps >&2 ;;
      esac
      ;;
    freebsd) print_freebsd_deps >&2 ;;
    macos) print_macos_deps >&2 ;;
    *) print_debian_deps >&2 ;;
    esac
    exit 1
  fi
}

clean_tree() (
  [[ -d "$PHP_SRC_DIR" ]] || die "PHP source directory missing: $PHP_SRC_DIR"
  cd "$PHP_SRC_DIR"
  if [[ -f Makefile ]]; then
    local make_command="${MAKE:-make}"
    if [[ "$(detect_os)" == freebsd ]]; then make_command="${MAKE:-gmake}"; fi
    "$make_command" clean
    "$make_command" distclean
    rm -f .claviron-php-build
  else
    log "No Makefile in $PHP_SRC_DIR; nothing to clean."
  fi
)

build_php() {
  local configure_args=()
  local iconv_configure_arg="--with-iconv"
  local bz2_configure_arg="--with-bz2"

  if [[ "$(detect_os)" == "freebsd" ]]; then
    # FreeBSD provides iconv in libc, while ports install GNU libiconv under
    # /usr/local. The ports include path exposes GNU headers, so select the
    # matching library explicitly instead of mixing them with libc iconv.
    iconv_configure_arg="--with-iconv=/usr/local"
  fi
  if [[ "$(detect_os)" == "macos" ]]; then
    # bzip2 has no pkg-config file; configure needs its keg's prefix.
    bz2_configure_arg="--with-bz2=$(brew --prefix bzip2)"
  fi

  sudo_cmd mkdir -p "$PHP_CONFIG_DIR" "$PHP_SCAN_ROOT" "$PHP_AVAILABLE_SCAN_DIR" "$PHP_EMBED_SCAN_DIR" "$PHP_CLI_SCAN_DIR"
  sudo_cmd chown -R "$(id -u):$(id -g)" "$PHP_DIR"

  configure_args=(
    --prefix="$PHP_DIR"
    "$PHP_DEBUG_OPTION"
    "$PHP_ASSERTIONS_OPTION"
    --with-config-file-path="$PHP_CONFIG_DIR"
    --with-config-file-scan-dir="$PHP_SCAN_DIR"
    --enable-embed=shared
    --disable-zend-signals
    --enable-zts
    --enable-cli
    --with-pear
    --enable-intl
    --enable-calendar
    --enable-mbstring
    --enable-bcmath
    --enable-exif
    --enable-fileinfo
    --enable-filter
    --enable-session
    --enable-tokenizer
    --enable-dom
    --enable-simplexml
    --enable-xml
    --enable-xmlreader
    --enable-xmlwriter
    --enable-phar
    --enable-pcntl
    --enable-posix
    --enable-shmop
    --enable-sysvmsg
    --enable-sysvsem
    --enable-sysvshm
    --enable-sockets
    --enable-gd
    --with-jpeg
    --with-webp
    --with-freetype
    --with-zlib
    "$bz2_configure_arg"
    --with-zip
    --with-curl
    "$iconv_configure_arg"
    --with-openssl
    --with-password-argon2
    --with-sodium
    --with-libxml
    --with-xsl
    --with-mysqli=mysqlnd
    --with-pdo-mysql=mysqlnd
    --with-pdo-pgsql
    --with-pgsql
    --with-pdo-sqlite
    --with-sqlite3
    --with-gmp
  )

  if [[ "$PHP_ENABLE_FPM" == "1" ]]; then
    configure_args+=(--enable-fpm)
  fi

  local configured_version="" fingerprint previous=""
  local stamp_file=.claviron-php-build
  command -v "${CC:-cc}" >/dev/null || die 'C compiler not found; set CC.'
  command -v "${CXX:-c++}" >/dev/null || die 'C++ compiler not found; set CXX.'
  fingerprint="$(
    printf '%s\n' "$PHP_BUILD_PROFILE" "$PHP_VERSION" "${configure_args[@]}" \
      "$CFLAGS" "$CXXFLAGS" "$CPPFLAGS" "$LDFLAGS" "$PKG_CONFIG_PATH" \
      "$(command -v "${CC:-cc}")" "$("${CC:-cc}" --version)" \
      "$(command -v "${CXX:-c++}")" "$("${CXX:-c++}" --version)" | cksum
  )"
  if [[ -f "$stamp_file" ]]; then previous="$(cat "$stamp_file")"; fi
  if [[ -f config.status ]]; then
    configured_version="$(sh ./config.status --version 2>/dev/null | sed -n '1p')"
  fi
  if [[ "$RECONFIGURE" == 1 || "$PHP_SOURCE_CHANGED" == 1 ||
        "$configured_version" != "PHP config.status $PHP_VERSION" ||
        "$previous" != "$fingerprint" ||
        ! -f Makefile || ! -f main/php_config.h || ! -f main/build-defs.h ]]; then
    # A failed configure must not look reusable on the next install attempt.
    rm -f config.status Makefile "$stamp_file"
    if ! ./buildconf --force || ! ./configure "${configure_args[@]}"; then
      rm -f config.status Makefile "$stamp_file"
      return 1
    fi
    # Use the new Makefile even when the old one was lost but objects survived.
    # Make does not track flags; no object from a previous profile may survive.
    "${MAKE:-make}" clean
    printf '%s\n' "$fingerprint" > "$stamp_file"
  else
    log "Reusing PHP $PHP_VERSION configuration; use install --configure to change build options."
  fi

  "${MAKE:-make}" -j"$(jobs_count)"
  "${MAKE:-make}" install
}

write_default_ini() {
  local ini="$PHP_CONFIG_DIR/php.ini"
  local ca_file="/etc/ssl/cert.pem"
  local private_ca="${PHP_PRIVATE_CA:-}"
  local ca_bundle="$PHP_CONFIG_DIR/ca-bundle.pem"
  local ca_ini="$PHP_AVAILABLE_SCAN_DIR/10-ca.ini"

  if [[ ! -f "$ini" ]]; then
    cp "$PHP_SRC_DIR/php.ini-production" "$ini"
  fi

  mkdir -p "$PHP_SCAN_ROOT" "$PHP_AVAILABLE_SCAN_DIR" "$PHP_EMBED_SCAN_DIR" "$PHP_CLI_SCAN_DIR"

  # Ports OpenSSL has a different compiled CA path from FreeBSD base.
  # Keep an operator-supplied private authority in the installation's CA bundle.
  if [[ "$(detect_os)" == "freebsd" && -r "$ca_file" ]]; then
    if [[ -n "$private_ca" ]]; then
      [[ -r "$private_ca" ]] || die "PHP_PRIVATE_CA is not readable: $private_ca"
      cat "$ca_file" "$private_ca" >"$ca_bundle"
      ca_file="$ca_bundle"
    fi
    printf 'openssl.cafile=%s\n' "$ca_file" >"$ca_ini"
    ln -sf "../available/$(basename "$ca_ini")" "$PHP_EMBED_SCAN_DIR/$(basename "$ca_ini")"
    ln -sf "../available/$(basename "$ca_ini")" "$PHP_CLI_SCAN_DIR/$(basename "$ca_ini")"
  fi
}

write_macos_fpm_defaults() (
  local global_config="$PHP_CONFIG_DIR/php-fpm.conf"
  local pool_dir="$PHP_CONFIG_DIR/php-fpm.d"
  local fpm_templates="$PHP_SRC_DIR/sapi/fpm"
  local config

  sudo_cmd mkdir -p -m 0755 "$PHP_CONFIG_DIR/conf.d/fpm"
  [[ ! -s "$global_config" && ! -L "$global_config" ]] || return 0
  sudo_cmd mkdir -p -m 0755 "$pool_dir"
  config="$(mktemp)"
  trap 'rm -f "$config"' EXIT

  shopt -s nullglob
  local pools=("$pool_dir"/*.conf)
  if ((${#pools[@]} == 0)); then
    sed -e 's|^listen = .*|listen = run/php-fpm.sock|' \
      -e 's/^;listen\.mode = .*/listen.mode = 0600/' \
      "$fpm_templates/www.conf" >"$config"
    sudo_cmd install -m 0644 "$config" "$pool_dir/www.conf"
  fi

  # The LaunchAgent supplies its own prefix for the socket and logs. Use an
  # absolute include so pool files still resolve in the configuration directory.
  sed '/^include=/d' "$fpm_templates/php-fpm.conf" >"$config"
  printf 'include="%s/*.conf"\n' "$pool_dir" >>"$config"
  sudo_cmd install -m 0644 "$config" "$global_config"
)

# The embed library's file name: macOS builds a dylib.
libphp_path() {
  if [[ "$(detect_os)" == "macos" ]]; then
    echo "$PHP_DIR/lib/libphp.dylib"
  else
    echo "$PHP_DIR/lib/libphp.so"
  fi
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

fix_runtime_permissions() {
  if account_exists passwd "$PHP_OWNER" && account_exists group "$PHP_GROUP"; then
    log "setting runtime ownership to $PHP_OWNER:$PHP_GROUP"
    sudo_cmd chown -R "$PHP_OWNER:$PHP_GROUP" "$PHP_DIR"
  else
    die "owner $PHP_OWNER:$PHP_GROUP does not exist; set PHP_OWNER and PHP_GROUP"
  fi

  if [[ "$(detect_os)" == macos ]]; then
    # Keep operator configuration permissions when preparing the user agent.
    sudo_cmd find "$PHP_DIR" \( -path "$PHP_CONFIG_DIR/php-fpm.d" -o -path "$PHP_CONFIG_DIR/php-fpm.conf" \) -prune \
      -o \( -type d -exec chmod 0755 {} + \) -o \( -type f -exec chmod u=rwX,go=rX {} + \)
  else
    sudo_cmd find "$PHP_DIR" -type d -exec chmod 0755 {} +
    sudo_cmd find "$PHP_DIR" -type f -exec chmod u=rwX,go=rX {} +
  fi
  sudo_cmd find "$PHP_DIR/bin" -type f -exec chmod 0755 {} +
  sudo_cmd find "$PHP_DIR/sbin" -type f -exec chmod 0755 {} + 2>/dev/null || true
  [[ -f "$(libphp_path)" ]] && sudo_cmd chmod 0755 "$(libphp_path)"
  # Every process that embeds or runs this PHP reads php.ini as its own user.
  # Named here because the configuration directory may lie outside PHP_DIR.
  sudo_cmd chmod 0644 "$PHP_CONFIG_DIR/php.ini"
}

# PHP starts without complaint when it cannot read php.ini and runs on its
# built-in defaults, so the file being present proves nothing: ask PHP.
require_loaded_ini() {
  local ini="$PHP_CONFIG_DIR/php.ini"
  local loaded

  [[ -f "$ini" ]] || die "php.ini is missing: $ini"
  [[ -n "$(find "$ini" -perm -0004 2>/dev/null)" ]] ||
    die "php.ini is not readable by other users: $ini (expected mode 0644)"
  loaded="$(PHPRC="$ini" PHP_INI_SCAN_DIR="$PHP_EMBED_SCAN_DIR" "$PHP_DIR/bin/php" -r 'echo (string) php_ini_loaded_file();')"
  [[ "$loaded" == "$ini" ]] ||
    die "PHP did not load $ini as $(id -un) (loaded: ${loaded:-none})"
}

register_runtime_library() {
  if [[ "$PHP_BUILD_PROFILE" == debug ]]; then
    log "Debug library stays private; select $PHP_DIR/lib explicitly when linking."
    return 0
  fi
  local conf="/etc/ld.so.conf.d/php-zts-embed.conf"

  if [[ "$(detect_os)" == "macos" ]]; then
    # macOS has no loader cache. PHP installs the dylib as @rpath/libphp.dylib;
    # giving it its absolute install name means every binary linking it
    # records the full path, so it loads without an rpath, a symlink or
    # DYLD_LIBRARY_PATH. Changing the header invalidates the signature, which
    # Apple silicon requires, so the dylib is signed again (ad hoc).
    local dylib
    dylib="$(libphp_path)"
    log "setting the install name of $dylib to its own path"
    sudo_cmd install_name_tool -id "$dylib" "$dylib"
    sudo_cmd codesign --force --sign - "$dylib"
    return 0
  fi

  if [[ "$(detect_os)" == "freebsd" ]]; then
    # FreeBSD ldconfig at boot scans /usr/local/libdata/ldconfig/* (per
    # default rc.conf `ldconfig_local_dirs`). Write the path here so it
    # survives reboots; `ldconfig -m` only updates the running cache
    # (volatile) and would otherwise be lost on next boot.
    local fbsd_conf="/usr/local/libdata/ldconfig/php-zts-embed"
    log "registering $PHP_DIR/lib in $fbsd_conf (persistent across reboots)"
    sudo_cmd mkdir -p /usr/local/libdata/ldconfig
    sudo_cmd chown root:wheel "$PHP_DIR/lib"
    printf '%s\n' "$PHP_DIR/lib" | sudo_cmd tee "$fbsd_conf" >/dev/null
    sudo_cmd ldconfig -m "$PHP_DIR/lib"
    return 0
  fi

  log "registering libphp.so path in $conf"
  printf '%s\n' "$PHP_DIR/lib" | sudo_cmd tee "$conf" >/dev/null
  sudo_cmd ldconfig
}

# Two separate questions. The install check decides whether this PHP is
# installed correctly, and fails the run if not. The configuration report reads
# this host's php.ini against the services it finds (database sockets, CA
# files); it only advises, because those belong to the host, not the install.
verify_install() {
  local cli_version
  local config_version

  INFO "Install check"
  [[ -x "$PHP_DIR/bin/php" ]] || die "php binary missing: $PHP_DIR/bin/php"
  [[ -x "$PHP_DIR/bin/php-config" ]] || die "php-config missing: $PHP_DIR/bin/php-config"
  [[ -f "$(libphp_path)" ]] || die "embed library missing: $(libphp_path)"
  if [[ "$(detect_os)" == "macos" && "$PHP_BUILD_PROFILE" != debug ]]; then
    [[ "$(otool -D "$(libphp_path)" | sed -n '2p')" == "$(libphp_path)" ]] ||
      die "$(libphp_path) does not carry its own path as its install name"
  fi
  php_zts_require_profile
  [[ "$("$PHP_DIR/bin/php" -n -r 'echo (int) PHP_ZTS;')" == 1 ]] || die 'Expected a ZTS PHP build.'
  require_loaded_ini

  PHPRC="$PHP_CONFIG_DIR/php.ini" PHP_INI_SCAN_DIR="$PHP_EMBED_SCAN_DIR" "$PHP_DIR/bin/php" -v
  "$PHP_DIR/bin/php-config" --php-sapis
  PHPRC="$PHP_CONFIG_DIR/php.ini" PHP_INI_SCAN_DIR="$PHP_EMBED_SCAN_DIR" "$PHP_DIR/bin/php" --ini
  cli_version="$(PHPRC="$PHP_CONFIG_DIR/php.ini" PHP_INI_SCAN_DIR="$PHP_EMBED_SCAN_DIR" "$PHP_DIR/bin/php" -r 'echo PHP_VERSION;')"
  config_version="$("$PHP_DIR/bin/php-config" --version)"
  if [[ "$cli_version" != "$PHP_VERSION" || "$config_version" != "$PHP_VERSION" ]]; then
    FAILED "Expected PHP $PHP_VERSION, got cli=$cli_version php-config=$config_version"
    exit 1
  fi
  SUCCESS "PHP release version verified ($PHP_VERSION)"

  PHPRC="$PHP_CONFIG_DIR/php.ini" PHP_INI_SCAN_DIR="$PHP_EMBED_SCAN_DIR" "$PHP_DIR/bin/php" -r '
    $gd = function_exists("gd_info") ? gd_info() : [];
    foreach (["JPEG Support", "PNG Support", "WebP Support", "FreeType Support"] as $key) {
        if (empty($gd[$key])) {
            fwrite(STDERR, "❌ \033[1;31m[FAILED]\033[0m PHP GD missing required capability: {$key}\n");
            exit(1);
        }
    }
    fwrite(STDERR, "\033[1;32m[✔]\033[0m PHP GD JPEG/PNG/WebP/FreeType support verified\n");
  '

  SUCCESS "PHP $PHP_VERSION ($PHP_BUILD_PROFILE) is installed correctly at $PHP_DIR"

  INFO "Configuration report for this host (advisory; it does not affect the installation)"
  local config_report="$PHP_INSTALL_SCRIPT_DIR/php-zts-verify-config.php"
  local config_failed=0
  if ! PHPRC="$PHP_CONFIG_DIR/php.ini" PHP_INI_SCAN_DIR="$PHP_EMBED_SCAN_DIR" "$PHP_DIR/bin/php" "$config_report" embed; then
    config_failed=1
  fi
  if [[ "$PHP_CLI_SCAN_DIR" != "$PHP_EMBED_SCAN_DIR" ]]; then
    if ! PHPRC="$PHP_CONFIG_DIR/php.ini" PHP_INI_SCAN_DIR="$PHP_CLI_SCAN_DIR" "$PHP_DIR/bin/php" "$config_report" cli; then
      config_failed=1
    fi
  fi
  if [[ "$config_failed" == 1 ]]; then
    WARNING "The configuration report found php.ini settings to review for this host (above). The installation itself is complete."
  else
    SUCCESS "The configuration report found nothing to review."
  fi
}

dry_run() {
  echo "OS: $(detect_os)"
  if [[ "$(detect_os)" == "linux" ]]; then
    echo "Linux family: $(detect_linux_family)"
  fi
  echo "php version: $PHP_VERSION"
  echo "build profile: $PHP_BUILD_PROFILE"
  echo "C compiler: ${CC:-cc}"
  local libphp_flags
  libphp_flags="$(php_zts_libphp_flags "$(detect_os)")"
  echo "CFLAGS: ${CFLAGS:+$CFLAGS }$PHP_PROFILE_FLAGS${libphp_flags:+ $libphp_flags}"
  echo "CXXFLAGS: ${CXXFLAGS:+$CXXFLAGS }$PHP_PROFILE_FLAGS${libphp_flags:+ $libphp_flags}"
  echo "debug option: $PHP_DEBUG_OPTION"
  echo "php-src: $PHP_SRC_DIR"
  echo "prefix: $PHP_DIR"
  echo "config dir: $PHP_CONFIG_DIR"
  echo "available scan dir: $PHP_AVAILABLE_SCAN_DIR"
  echo "embed scan dir: $PHP_EMBED_SCAN_DIR"
  echo "cli scan dir: $PHP_CLI_SCAN_DIR"
  echo "make jobs: $(jobs_count)"
  echo "skip package install: $SKIP_APT"
  echo "dependencies:"
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
}

usage() {
  cat <<EOF
Usage: $0 [help|install [--configure]|clean|verify|dry-run|check-source|deps]

  help          Show this help (default; also -h or --help).
  install       Build and install using the existing configuration.
  install --configure
                Run buildconf --force and configure before building and installing.
                Use when changing configure options, compiler or build environment.
                Discards objects compiled with the previous settings.
  clean         Run make clean and make distclean in the PHP source directory.
  verify        Check the installed PHP without rebuilding.
  dry-run       Show resolved installation paths and dependencies.
  check-source  Prepare the PHP source release tag only.
  deps          Print package dependencies only.

First installation, a source revision change, a configured PHP version mismatch,
or missing configuration outputs automatically runs buildconf and configure.
Compiler, flag, profile and configure-option changes also trigger a clean
reconfiguration. Retry a failed make with unchanged settings to reuse completed
objects. Use install --configure to force a clean reconfiguration.

PHP_BUILD_PROFILE=release (default): -O2 -g0, PHP debug and debug assertions off;
  on Linux and FreeBSD libphp also gets -fno-semantic-interposition and
  -ftls-model=initial-exec (not on macOS, where neither applies).
PHP_BUILD_PROFILE=debug: -O0 -g, PHP debug enabled, separate source and prefix.
Debug defaults to /opt/php-zts-embed-debug and is not registered with the
loader (ldconfig, or the install name on macOS).
On macOS the build dependencies come from Homebrew; SKIP_APT=1 skips installing
them, as it skips apt, dnf and pkg elsewhere.
CC and CXX select the compiler; inherited include/link flags are preserved.
Xdebug is an independent, explicitly installed CLI extension.

Installation must be explicitly requested. For a clean installation build:
  $0 clean && $0 install

To check an existing installation without rebuilding:
  $0 verify
EOF
}

main() {
  case "${1:-help}" in
    help|-h|--help) usage; return 0 ;;
  esac

  if [[ "${1:-}" == install && "${2:-}" == --configure && $# == 2 ]]; then
    RECONFIGURE=1
  elif [[ $# -gt 1 ]]; then
    die "usage: $0 install [--configure]; other commands take no arguments"
  fi

  local os
  os="$(detect_os)"

  case "${1:-help}" in
  deps)
    case "$os" in
    linux)
      case "$(detect_linux_family)" in
      rhel) print_almalinux_deps ;;
      *) print_debian_deps ;;
      esac
      ;;
    freebsd) print_freebsd_deps ;;
    macos) print_macos_deps ;;
    esac
    exit 0
    ;;
  dry-run)
    dry_run
    exit 0
    ;;
  check-source)
    prepare_source
    exit 0
    ;;
  verify)
    verify_install
    exit 0
    ;;
  clean)
    clean_tree
    return 0
    ;;
  install) ;;
  *) usage >&2; die "Unknown command: $1" ;;
  esac

  log "building PHP ZTS embed SAPI"
  log "php-src: $PHP_SRC_DIR"
  log "prefix: $PHP_DIR"
  log "build profile: $PHP_BUILD_PROFILE ($PHP_PROFILE_FLAGS, $PHP_DEBUG_OPTION)"
  php_zts_require_profile

  prepare_source
  configure_build_env "$os"
  verify_re2c
  verify_pkg_config
  build_php
  write_default_ini
  if [[ "$os" == macos && "$PHP_ENABLE_FPM" == 1 ]]; then
    write_macos_fpm_defaults
  fi
  fix_runtime_permissions
  register_runtime_library
  verify_install
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then main "$@"; fi
