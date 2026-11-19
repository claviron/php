#!/usr/bin/env bash
set -euo pipefail

# PHP-FPM on macOS as a LaunchAgent of the active user: it runs as that user,
# through the same launcher (php-zts-fpm.sh) systemd uses on Linux, so the
# configuration is checked before every start.
#
# The PHP installation under /opt is root-owned and only read. What FPM
# writes stays with the user:
#   ~/Library/Application Support/php-zts-fpm   FPM's --prefix: relative pid and
#                                               log paths in php-fpm.conf resolve
#                                               here (run/, log/), and the launcher
#   ~/Library/Logs/php-zts-fpm.log               the service's standard output
#
# PHP_INSTALL_DIR (default /opt/php-zts-embed),
# PHP_CONFIG_DIR (default: its etc) and PHP_INI_SCAN_DIR (default:
# PHP_CONFIG_DIR/conf.d/fpm) choose the installation and the php.ini,
# php-fpm.conf and INI directory FPM uses.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
SERVICE_NAME="php-zts-fpm"
PLIST_TEMPLATE="$SCRIPT_DIR/${SERVICE_NAME}.plist"
PLIST_DEST="$HOME/Library/LaunchAgents/${SERVICE_NAME}.plist"
STATE_DIR="$HOME/Library/Application Support/$SERVICE_NAME"
LAUNCHER="$STATE_DIR/php-zts-fpm"
LOG_FILE="$HOME/Library/Logs/${SERVICE_NAME}.log"
LAUNCHCTL_DOMAIN="gui/$(id -u)"
LAUNCHCTL_TARGET="$LAUNCHCTL_DOMAIN/${SERVICE_NAME}"

usage() {
  cat <<EOF
Usage: $(basename "$0") install
       $(basename "$0") <start|stop|restart|reload|status|watch|uninstall>

  install    Check the installation and configuration, write the LaunchAgent
             for the active user and start it
  start      Start the service
  stop       Stop the service
  restart    Restart the service
  reload     Restart the loaded service with launchctl kickstart -k
  status     Show whether the service is loaded and running
  watch      Follow the service log ($LOG_FILE)
  uninstall  Stop the service and remove the LaunchAgent

PHP_INSTALL_DIR, PHP_CONFIG_DIR and PHP_INI_SCAN_DIR choose the installation
and the files FPM uses; see the head of this script for the defaults.
EOF
}

die() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

service_exists() {
  launchctl print "$LAUNCHCTL_TARGET" >/dev/null 2>&1
}

install_service() {
  [[ "$(uname -s)" == Darwin ]] || die "this script is for macOS; use php-zts-fpm-install.sh on Linux"
    export PHP_INSTALL_DIR="${PHP_INSTALL_DIR:-/opt/php-zts-embed}"
  export PHP_CONFIG_DIR="${PHP_CONFIG_DIR:-$PHP_INSTALL_DIR/etc}"
  export PHP_INI_SCAN_DIR="${PHP_INI_SCAN_DIR:-$PHP_CONFIG_DIR/conf.d/fpm}"
  export PHP_FPM_PREFIX="$STATE_DIR"
  [[ -f "$PLIST_TEMPLATE" ]] || die "missing plist template: $PLIST_TEMPLATE"

  mkdir -p "$STATE_DIR/run" "$STATE_DIR/log" "$(dirname "$LOG_FILE")" "$(dirname "$PLIST_DEST")"
  install -m 0755 "$SCRIPT_DIR/php-zts-fpm.sh" "$LAUNCHER"
  "$LAUNCHER" check

  local environment
  environment="$(printf '{"PHP_INSTALL_DIR":"%s","PHP_CONFIG_DIR":"%s","PHP_INI_SCAN_DIR":"%s","PHP_FPM_PREFIX":"%s"}' \
    "$PHP_INSTALL_DIR" "$PHP_CONFIG_DIR" "$PHP_INI_SCAN_DIR" "$PHP_FPM_PREFIX")"
  cp "$PLIST_TEMPLATE" "$PLIST_DEST"
  plutil -replace ProgramArguments -json "[\"$LAUNCHER\",\"start\"]" "$PLIST_DEST"
  plutil -replace EnvironmentVariables -json "$environment" "$PLIST_DEST"
  plutil -replace StandardErrorPath -string "$LOG_FILE" "$PLIST_DEST"
  plutil -replace StandardOutPath -string "$LOG_FILE" "$PLIST_DEST"
  plutil -replace WorkingDirectory -string "$STATE_DIR" "$PLIST_DEST"
  plutil -lint "$PLIST_DEST" >/dev/null

  launchctl bootout "$LAUNCHCTL_DOMAIN" "$PLIST_DEST" 2>/dev/null || true
  launchctl bootstrap "$LAUNCHCTL_DOMAIN" "$PLIST_DEST"
  launchctl enable "$LAUNCHCTL_TARGET"
  launchctl kickstart -k "$LAUNCHCTL_TARGET"
  echo "Installed and started: $SERVICE_NAME (as $(id -un))"
}

start_service() {
  [[ -f "$PLIST_DEST" ]] || die "missing LaunchAgent $PLIST_DEST; run: $(basename "$0") install"
  if ! service_exists; then
    launchctl bootstrap "$LAUNCHCTL_DOMAIN" "$PLIST_DEST"
    launchctl enable "$LAUNCHCTL_TARGET"
  fi
  launchctl kickstart -k "$LAUNCHCTL_TARGET"
  echo "Started: $SERVICE_NAME"
}

stop_service() {
  if service_exists; then
    launchctl bootout "$LAUNCHCTL_DOMAIN" "$PLIST_DEST"
    echo "Stopped: $SERVICE_NAME"
  else
    echo "Service not loaded: $SERVICE_NAME"
  fi
}

restart_service() {
  stop_service
  start_service
}

reload_service() {
  service_exists || die "service not loaded: $SERVICE_NAME"
  launchctl kickstart -k "$LAUNCHCTL_TARGET"
  echo "Reloaded: $SERVICE_NAME"
}

status_service() {
  if ! service_exists; then
    echo "Status: not loaded"
    exit 3
  fi

  local state pid last_exit_status
  state="$(launchctl print "$LAUNCHCTL_TARGET" 2>/dev/null | awk -F ' = ' '/state = / { print $2; exit }')"
  pid="$(launchctl print "$LAUNCHCTL_TARGET" 2>/dev/null | awk -F ' = ' '/pid = / { print $2; exit }')"
  last_exit_status="$(launchctl print "$LAUNCHCTL_TARGET" 2>/dev/null | awk -F ' = ' '/last exit code = / { print $2; exit }')"

  if [[ -n "$pid" ]]; then
    echo "Status: running"
    echo "PID: $pid"
    [[ -n "$state" ]] && echo "State: $state"
    exit 0
  fi

  echo "Status: loaded but not running"
  [[ -n "$state" ]] && echo "State: $state"
  [[ -n "$last_exit_status" ]] && echo "Last exit code: $last_exit_status"
  exit 1
}

watch_service() {
  tail -f "$LOG_FILE"
}

uninstall_service() {
  if service_exists; then
    launchctl bootout "$LAUNCHCTL_DOMAIN" "$PLIST_DEST"
  fi
  rm -f "$PLIST_DEST"
  echo "Uninstalled: $SERVICE_NAME (run and log files under $STATE_DIR are kept)"
}

main() {
  case "${1:-}" in
  install) install_service ;;
  start) start_service ;;
  stop) stop_service ;;
  restart) restart_service ;;
  reload) reload_service ;;
  status) status_service ;;
  watch) watch_service ;;
  uninstall) uninstall_service ;;
  -h | --help) usage ;;
  *)
    usage
    exit 1
    ;;
  esac
}

main "$@"
