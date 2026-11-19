# PHP installation

These scripts own the shared ZTS PHP toolchain and FPM service helpers.
The PHP builder and the module installer support Linux, FreeBSD and macOS with
one script each: the build is the same everywhere, and only the source of the
build dependencies differs (apt or dnf, pkg, Homebrew). On macOS the installed
`libphp.dylib` carries its own absolute install name, so binaries linking it
need no rpath. The macOS FPM helper manages the installed runtime through
launchd.

Run commands from the root of this repository. The PHP builder, module installer
and FPM installers print help without arguments. Building, installing extensions
and installing service files are separate operations. Choose a supported PHP
release explicitly when upgrading.

## Files

| File | What it is | Platforms |
| --- | --- | --- |
| `php-zts-install.sh` | Builds and installs ZTS PHP with the embed SAPI and FPM from the official php-src tag; `deps`, `dry-run`, `verify`, `install` | Linux, FreeBSD, macOS |
| `php-zts-install-modules.sh` | Installs PECL extensions against that PHP and links them into the embed and CLI scan directories; ends with whether every requested extension loaded | Linux, FreeBSD, macOS |
| `build-profile.sh` | The `release` and `debug` profiles (compiler flags, prefix, source directory); sourced by the two scripts above, not run itself | all |
| `php-zts-verify-config.php` | The advisory configuration report `php-zts-install.sh` prints after its install check; it reports and never fails the install | all |
| `php-zts-fpm.sh` | The FPM launcher a service manager runs: checks the version and configuration, then `check` (`php-fpm --test`) or `start` | Linux, macOS |
| `php-zts-fpm-install.sh` | Installs the systemd unit, the launcher, the logrotate entry and the service environment file; does not start FPM | Linux |
| `php-zts-fpm.service` | The systemd unit it installs | Linux |
| `php-zts-fpm.logrotate` | The logrotate entry it installs | Linux |
| `php-zts-fpm-install-freebsd.sh` | Installs the rc.d service from the native rc configuration; does not start FPM | FreeBSD |
| `php-zts-fpm-macos.sh` | Installs FPM as a LaunchAgent of the active user and controls it: `install`, `start`, `stop`, `restart`, `reload`, `status`, `watch`, `uninstall` | macOS |
| `php-zts-fpm.plist` | The LaunchAgent template `php-zts-fpm-macos.sh install` fills in | macOS |
| `composer-install.sh` | Installs Composer after verifying its official installer's signature | all |

## Build profiles

| Profile | Compiler flags | PHP debug / assertions | Default prefix | Default source |
| --- | --- | --- | --- | --- |
| `release` (default) | `-O2 -g0`; libphp on Linux and FreeBSD also `-fno-semantic-interposition -ftls-model=initial-exec` | Disabled | `/opt/php-zts-embed` | `$HOME/src/php-src` |
| `debug` | `-O0 -g` | Enabled | `/opt/php-zts-embed-debug` | `$HOME/src/php-src-debug` |

```sh
scripts/install/php-zts-install.sh dry-run
PHP_VERSION=8.5.13 scripts/install/php-zts-install.sh install

PHP_BUILD_PROFILE=debug PHP_VERSION=8.5.13 \
  scripts/install/php-zts-install.sh install
```

The release build is for production, tuned for how Claviron uses PHP: a host
links libphp at startup and runs ZTS requests in its own threads. On ELF
systems (Linux, FreeBSD) libphp is built with two more flags:

- `-fno-semantic-interposition`: libphp is a shared library, and without this
  its calls into itself go through the PLT and cannot be inlined.
- `-ftls-model=initial-exec`: ZTS reads its per-request thread-local state
  constantly; this model makes that one register offset instead of a
  `__tls_get_addr` call. It is valid because hosts load libphp at startup,
  not with `dlopen`.

macOS gets neither: Mach-O symbols are not interposable, and its thread locals
use a single mechanism. PECL extensions never get them, because PHP loads them
with `dlopen`. Two further steps are planned and will be measured before they
become defaults: profile-guided optimization (php-src's `make prof-gen` and
`make prof-use` with a training workload), and `-march=native`, which ties the
binary to the CPU it was built on and therefore stays opt-in.

`CC` and `CXX` select the compiler. The profile flags are appended to inherited
`CFLAGS` and `CXXFLAGS`, so include-only flags cannot remove optimization.
Use `CPPFLAGS` for dependency include directories and `LDFLAGS` for linker
options. `PHP_INSTALL_DIR`, `PHP_SRC_DIR`, `MAKE_JOBS` and `SKIP_APT=1` select
installation paths, build parallelism and preinstalled dependencies.

The builder records compiler identity, flags, profile and configure options.
Changes discard old object files before reconfiguration; an unchanged retry
after a failed compilation reuses completed objects. `install --configure`
forces a clean reconfiguration. The installed `PHP_DEBUG` and ZTS settings are
verified. An existing installation with a different debug profile is rejected.

Debug builds use separate source and installation directories and do not
register their library with the system dynamic linker. Select their matching
`PHP`, `PHP_CONFIG` and library search path explicitly in a development
application; do not mix release and debug extension binaries.

## Extensions and Xdebug

PHP's debug build adds engine diagnostics. Xdebug is a separately compiled
extension for PHP debugging, profiling and related tools; using Xdebug does
not require a PHP debug build. Neither default installer command installs it.
See the [Xdebug installation documentation](https://xdebug.org/docs/install).

```sh
scripts/install/php-zts-install-modules.sh install
scripts/install/php-zts-install-modules.sh install-xdebug

PHP_BUILD_PROFILE=debug \
  scripts/install/php-zts-install-modules.sh install-xdebug
```

The module installer uses the matching profile and prefix, checks the PHP ABI,
and keeps Xdebug in the CLI scan directory. Embedded requests do not load it.
PECL builds use an invocation-owned temporary directory. Module installation
does not remove other builds' temporary files.

## FPM service helpers

Prepare the selected installation's `php.ini`, `php-fpm.conf`, pools and FPM
INI scan directory before installing a service helper; the PHP builder does
not create them. Preserve existing socket paths and pool identities when
replacing a service.

On Linux and macOS the service starts FPM through one launcher,
`php-zts-fpm.sh`. Before every start it checks that the installation is the
expected `PHP_VERSION` and that `php.ini`, `php-fpm.conf` and the scan
directory exist. The service environment chooses them: `PHP_INSTALL_DIR`,
`PHP_CONFIG_DIR` (the directory holding `php.ini` and `php-fpm.conf`) and
`PHP_INI_SCAN_DIR`. `PHP_FPM_PREFIX`, when set, is FPM's `--prefix`, under
which relative pid and log paths in `php-fpm.conf` resolve.

Linux with systemd:

```sh
sudo env PHP_VERSION=8.5.13 \
  scripts/install/php-zts-fpm-install.sh install
```

This validates the selected runtime and configuration, then installs the unit,
the launcher, a logrotate entry and the service environment file:
`/etc/sysconfig/php-zts-fpm` on RHEL-family systems, `/etc/default/php-zts-fpm`
on Debian-family ones. It does not enable, start, stop or restart FPM, and it
leaves the system's `php` command alone.

FreeBSD:

```sh
sudo env PHP_VERSION=8.5.13 \
  scripts/install/php-zts-fpm-install-freebsd.sh install
```

This reads `php_zts_fpm` settings from the native rc configuration, validates
the runtime, and preserves an existing rc.d service file. It does not change
rc settings or restart FPM. The shared PHP builder selects `gmake` and the
FreeBSD compiler and dependency paths. The generated service uses graceful
`SIGQUIT` for stopping. Its log directory ownership is configured through
`php_zts_fpm_log_owner` and `php_zts_fpm_log_group` (defaults: `root`, `wheel`).

macOS, as a LaunchAgent of the active user:

```sh
PHP_VERSION=8.5.13 scripts/install/php-zts-fpm-macos.sh install
```

FPM runs as that user. The installation under `/opt/php-zts-embed` stays
root-owned and is only read; what FPM writes stays with the user: its prefix
(relative pid and log paths) in `~/Library/Application Support/php-zts-fpm`,
and the service output in `~/Library/Logs/php-zts-fpm.log`. Install checks
the configuration before it starts FPM. `start`, `stop`, `restart`, `reload`,
`status`, `watch` and `uninstall` manage it; use `--help` first.

When the distribution's re2c is older than PHP requires, install a newer one
and put it on `PATH`; the builder stops with an error otherwise.

## Composer

`scripts/install/composer-install.sh install` verifies the signature of
Composer's official installer and installs its PHAR for the selected PHP,
without rebuilding PHP.
