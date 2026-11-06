use super::*;
#[cfg(feature = "embed")]
use claviron_pipeline::Site;

#[cfg(feature = "embed")]
#[test]
fn file_routes_php_preparation_uses_the_same_mapping_as_files() {
    let directory = tempfile::tempdir().unwrap();
    let config: Site = serde_yaml::from_str(&format!(
        "root: {}\nroutes:\n  /admin:\n    directory: mapped\n    php: {{runner: script, entry_script: index.php}}\n  /plain:\n    php: {{runner: script}}\n",
        serde_yaml::to_string(directory.path()).unwrap().trim(),
    )).unwrap();
    let state =
        claviron_runtime::AppState::standalone("php-mapping", directory.path());
    for (prefix, disk) in [("/admin", "mapped"), ("/plain", "plain")] {
        let route = config.routes.find(prefix).unwrap();
        let declared = declarations(route).unwrap();
        let runtime = prepare_runtimes(&state, route, &declared).pop().unwrap();
        assert_eq!(runtime.dir, directory.path().join(disk));
        assert_eq!(
            runtime.dir,
            route.files.mapping.as_ref().unwrap().directory()
        );
        assert_eq!(
            runtime.relative_path(&format!("{prefix}/index.php")),
            Some("index.php")
        );
        assert_eq!(
            runtime.script_relative_path("/index.php", true),
            Some("index.php")
        );
    }
}

#[test]
fn absent_php_directives_prepare_no_execution_owner() {
    let state = AppState::standalone("php-declaration", ".");
    let prefix = claviron_http::ext::PathPrefix::root();
    let transport =
        claviron_http::config::transport::TransportPolicy::default();
    for source in ["{}", "php: null"] {
        let site: claviron_pipeline::Site =
            serde_yaml::from_str(source).unwrap();
        let context = BackendContext {
            state: &state,
            host: None,
            site: &site,
            route: &site,
            prefix: &prefix,
            policy: claviron_http::policy::HttpPolicy::unlimited(),
            transport: &transport,
            protocol: None,
            listener: None,
        };
        assert!(PhpFactory.prepare(&context).unwrap().is_empty());
    }
}

#[test]
fn php_directives_require_an_explicit_available_runner() {
    for source in ["php: {}", "php: {runner: unsupported}"] {
        let site: claviron_pipeline::Site =
            serde_yaml::from_str(source).unwrap();
        assert!(declarations(&site).is_err());
    }
}
