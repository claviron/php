//! Persistent HTTP applications: local capacity only, never fleet slot leases.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use claviron_app::{ApplicationDescription, ApplicationRetryPolicy};

use super::validate_program;

/// A boot script implementing the explicit application receive/respond protocol.
/// Every serving process starts the declared fixed instances. No PHP objects
/// cross threads, and HTTP demand never creates or retires an instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhpApplication {
    pub(crate) description: ApplicationDescription,
    /// Unresolved metadata intent; Some(empty) suppresses Composer discovery.
    pub(crate) frameworks: Option<Vec<String>>,
    pub(crate) name: String,
    pub(crate) script_path: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) env_file: Vec<PathBuf>,
    pub(crate) instances: u8,
    max_queued: Option<usize>,
    pub(crate) execution_timeout: Option<Duration>,
    pub(crate) retry: ApplicationRetryPolicy,
}

impl PhpApplication {
    pub fn new(
        name: impl Into<String>,
        script: impl Into<PathBuf>,
    ) -> Self {
        Self {
            description: ApplicationDescription::new("php", "application")
                .runner("claviron"),
            frameworks: None,
            name: name.into(),
            script_path: script.into(),
            args: Vec::new(),
            environment: BTreeMap::new(),
            env_file: Vec::new(),
            instances: default_instances(),
            max_queued: None,
            execution_timeout: None,
            retry: ApplicationRetryPolicy::new(
                default_backoff(),
                default_backoff_max(),
                None,
            ),
        }
    }
    /// Fixed instance count for this application in each serving process.
    pub fn instances(
        mut self,
        instances: u8,
    ) -> Self {
        self.instances = instances;
        self
    }
    /// Prepared management metadata; execution identity and mode stay engine-owned.
    pub fn framework(
        self,
        framework: impl Into<String>,
    ) -> Self {
        self.frameworks([framework.into()])
    }

    pub fn frameworks(
        mut self,
        frameworks: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let frameworks: Vec<String> =
            frameworks.into_iter().map(Into::into).collect();
        self.description.frameworks = frameworks.clone();
        self.frameworks = Some(frameworks);
        self
    }

    pub fn icon(
        mut self,
        icon: impl Into<String>,
    ) -> Self {
        self.description.icon = Some(icon.into());
        self
    }

    pub fn runner(
        mut self,
        runner: impl Into<String>,
    ) -> Self {
        self.description.runner = Some(runner.into());
        self
    }
    pub fn args(
        mut self,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }
    pub fn environment(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.environment.insert(key.into(), value.into());
        self
    }
    pub fn environments(
        mut self,
        values: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        self.environment.extend(values);
        self
    }
    /// Declare environment files without reading them during registration.
    /// Paths and inline overrides remain separate execution inputs.
    pub fn env_files(
        mut self,
        paths: impl IntoIterator<Item = impl Into<PathBuf>>,
    ) -> Self {
        self.env_file.extend(paths.into_iter().map(Into::into));
        self
    }
    pub fn validate(&self) -> Result<(), String> {
        validate_program(
            "PHP application",
            &self.name,
            &self.script_path,
            &self.args,
            &self.environment,
        )?;
        if self.instances == 0 {
            return Err("PHP application instances must be at least 1".into());
        }
        if self.admission_capacity().is_none() {
            return Err("PHP application instances + max_queued exceeds the supported capacity".into());
        }
        self.retry.validate("retry")
    }

    /// Maximum callers that can wait for an instance. Zero disables waiting.
    pub fn max_queued(
        mut self,
        value: usize,
    ) -> Self {
        self.max_queued = Some(value);
        self
    }

    pub(crate) fn admission_capacity(&self) -> Option<usize> {
        let instances = usize::from(self.instances);
        let queued = self.max_queued.unwrap_or(DEFAULT_ADMITTED - instances);
        instances
            .checked_add(queued)
            .filter(|&capacity| capacity <= tokio::sync::Semaphore::MAX_PERMITS)
    }

    /// Execution budget for one request or event; None uses the runtime policy.
    pub fn execution_timeout(
        mut self,
        value: Option<Duration>,
    ) -> Self {
        self.execution_timeout = value;
        self
    }

    /// Replacement policy for failed instances. Accepted work is never replayed.
    pub fn retry(
        mut self,
        value: ApplicationRetryPolicy,
    ) -> Self {
        self.retry = value;
        self
    }
}

pub(crate) fn default_instances() -> u8 {
    1
}
const DEFAULT_ADMITTED: usize = 256;
fn default_backoff() -> Duration {
    Duration::from_secs(1)
}
fn default_backoff_max() -> Duration {
    Duration::from_secs(60)
}

#[cfg(test)]
#[path = "../../tests/application.rs"]
mod tests;
