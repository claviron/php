//! INI locations are process-owned, even when a config evaluator starts PHP
//! before the HTTP/application providers boot.

use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
/// One process's PHP INI locations; not per document or per request settings.
/// Either one left unset is not decided here: PHP then takes `PHPRC` or
/// `PHP_INI_SCAN_DIR` from the process environment, or its compiled-in path.
pub struct PhpIni {
    /// `PHPRC`: the php.ini file, or the directory holding it.
    pub path: Option<PathBuf>,
    /// `PHP_INI_SCAN_DIR`. Empty declares that PHP scans no directory, PHP's
    /// own meaning of an empty value.
    pub scan_dir: Option<PathBuf>,
}

impl PhpIni {
    /// The locations PHP is told to read, which therefore have to exist.
    fn located(&self) -> impl Iterator<Item = &PathBuf> {
        self.path.iter().chain(
            self.scan_dir.iter().filter(|dir| !dir.as_os_str().is_empty()),
        )
    }
}

#[derive(Default)]
struct Declaration {
    ini: Option<PhpIni>,
    started: bool,
}

static DECLARATION: Mutex<Declaration> =
    Mutex::new(Declaration { ini: None, started: false });

/// Cold composition only: publish before any PHP execution or foreign threads.
/// A later consumer cannot silently change settings already used by PHP.
pub(crate) fn declare(ini: PhpIni) -> Result<(), String> {
    let mut declaration =
        DECLARATION.lock().map_err(|error| error.to_string())?;
    match &declaration.ini {
        Some(existing) if existing == &ini => return Ok(()),
        Some(_) => {
            return Err(
                "PHP INI locations already declared for this process".into()
            );
        }
        None if declaration.started && ini != PhpIni::default() => {
            return Err(
                "PHP already started before these INI locations were declared"
                    .into(),
            );
        }
        None => {}
    }
    if ini.path.as_ref().is_some_and(|path| path.as_os_str().is_empty())
        || [&ini.path, &ini.scan_dir]
            .into_iter()
            .flatten()
            .any(|path| path.as_os_str().as_encoded_bytes().contains(&0))
    {
        return Err(
            "PHP INI locations must be paths without NUL bytes, and the INI path nonempty"
                .into(),
        );
    }
    for (name, path) in
        [("PHPRC", &ini.path), ("PHP_INI_SCAN_DIR", &ini.scan_dir)]
    {
        if let Some(path) = path {
            // SAFETY: composition declares these before starting PHP/hosted
            // execution. Evaluation never changes the process environment.
            unsafe { std::env::set_var(name, path) };
        }
    }
    declaration.ini = Some(ini);
    Ok(())
}

pub(crate) fn start() -> Result<(), String> {
    let mut declaration =
        DECLARATION.lock().map_err(|error| error.to_string())?;
    if let Some(ini) = &declaration.ini {
        for path in ini.located() {
            if !path.exists() {
                return Err(format!(
                    "PHP INI path does not exist: {}",
                    path.display()
                ));
            }
        }
    }
    declaration.started = true;
    Ok(())
}
