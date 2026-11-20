#!/usr/bin/env bash
set -euo pipefail

# Start or check PHP-FPM for a service manager (systemd on Linux, launchd on
# macOS). The service environment names the installation and the files to
# use; nothing is assumed. Whatever release is installed there runs, so an
# upgrade needs only a restart:
#   PHP_INSTALL_DIR   the PHP installation
#   PHP_CONFIG_DIR    the directory holding php.ini and php-fpm.conf
#   PHP_INI_SCAN_DIR  the INI directory FPM scans (default: PHP_CONFIG_DIR/conf.d/fpm)
#   PHP_FPM_PREFIX    optional: FPM's --prefix, under which relative pid and
#                     log paths in php-fpm.conf resolve; a service that runs
#                     as an ordinary user points it at a directory it owns.

: "${PHP_INSTALL_DIR:?Set PHP_INSTALL_DIR in the service environment}"
: "${PHP_CONFIG_DIR:?Set PHP_CONFIG_DIR in the service environment}"
export PHPRC="$PHP_CONFIG_DIR/php.ini"
export PHP_INI_SCAN_DIR="${PHP_INI_SCAN_DIR:-$PHP_CONFIG_DIR/conf.d/fpm}"

# Name what is wrong: a file that exists but cannot be read by this user is a
# different repair from a file that was never written.
config_problems=()
for config_file in "$PHPRC" "$PHP_CONFIG_DIR/php-fpm.conf"; do
  if [[ ! -e "$config_file" ]]; then
    config_problems+=("missing: $config_file")
  elif [[ ! -r "$config_file" ]]; then
    config_problems+=("not readable by $(id -un): $config_file")
  fi
done
[[ -d "$PHP_INI_SCAN_DIR" ]] ||
  config_problems+=("missing scan directory: $PHP_INI_SCAN_DIR")
if ((${#config_problems[@]})); then
  printf 'PHP-FPM cannot start with the configuration in %s:\n' "$PHP_CONFIG_DIR" >&2
  printf '  %s\n' "${config_problems[@]}" >&2
  exit 1
fi

prefix_args=()
if [[ -n "${PHP_FPM_PREFIX:-}" ]]; then
  [[ -d "$PHP_FPM_PREFIX" && -w "$PHP_FPM_PREFIX" ]] || {
    printf 'PHP_FPM_PREFIX is not a writable directory: %s\n' "$PHP_FPM_PREFIX" >&2
    exit 1
  }
  prefix_args=(--prefix "$PHP_FPM_PREFIX")
fi

case "${1:-start}" in
  check) mode=--test ;;
  start) mode=--nodaemonize ;;
  *) printf 'Usage: php-zts-fpm [check|start]\n' >&2; exit 2 ;;
esac
exec "$PHP_INSTALL_DIR/sbin/php-fpm" "$mode" ${prefix_args[@]+"${prefix_args[@]}"} \
  -c "$PHPRC" --fpm-config "$PHP_CONFIG_DIR/php-fpm.conf"
