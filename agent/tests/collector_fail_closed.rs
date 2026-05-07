/// Fail-closed tests for the collector framework.
///
/// Each test creates a temporary collector root directory with a collector
/// subdirectory containing shell scripts that write marker files on execution.
use lightai_agent::collector::registry::RegistryEntry;
use lightai_agent::gpu;
use std::path::PathBuf;

struct TmpCollector {
    root: PathBuf,
    dir: PathBuf,
}

fn tmp_collector(name: &str, id: &str, version: &str) -> TmpCollector {
    let root = std::env::temp_dir().join(format!("lightai-coll-root-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let dir = root.join(id);
    std::fs::create_dir_all(&dir).unwrap();

    let toml = format!(
        "id = \"{id}\"\nvendor = \"test\"\nname = \"Test\"\nversion = \"{version}\"\n\
         description = \"test\"\ndiscover = \"discover.sh\"\nmetrics = \"metrics.sh\"\n\
         enabled = true\npriority = 100\n"
    );
    std::fs::write(dir.join("collector.toml"), toml).unwrap();

    let marker_d = dir.join(".discover-ran");
    let marker_m = dir.join(".metrics-ran");

    let ds = format!(
        "#!/bin/sh\ntouch \"{}\"\nprintf 'STATUS\\t1\\tok\\ttest\\t{id}\\t\\n'\n",
        marker_d.display()
    );
    let dp = dir.join("discover.sh");
    std::fs::write(&dp, ds).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dp, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let ms = format!(
        "#!/bin/sh\ntouch \"{}\"\nprintf 'STATUS\\t1\\tok\\ttest\\t{id}\\t\\n'\n",
        marker_m.display()
    );
    let mp = dir.join("metrics.sh");
    std::fs::write(&mp, ms).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&mp, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    TmpCollector { root, dir }
}

fn markers(tc: &TmpCollector) -> (PathBuf, PathBuf) {
    (tc.dir.join(".discover-ran"), tc.dir.join(".metrics-ran"))
}

fn sha256_file(path: &PathBuf) -> String {
    lightai_agent::collector::execute::sha256_hex(&std::fs::read(path).unwrap())
}

fn registry_entry(id: &str, version: &str, tc: &TmpCollector, enabled: bool) -> RegistryEntry {
    let now = 1700000000;
    RegistryEntry {
        id: id.to_string(),
        vendor: "test".to_string(),
        name: "Test".to_string(),
        version: version.to_string(),
        description: String::new(),
        discover_sha256: sha256_file(&tc.dir.join("discover.sh")),
        metrics_sha256: sha256_file(&tc.dir.join("metrics.sh")),
        enabled,
        created_at: now,
        updated_at: now,
    }
}

fn make_config(tc: &TmpCollector, registry: Vec<RegistryEntry>) -> gpu::CollectorConfig {
    gpu::CollectorConfig {
        collector_root: Some(tc.root.clone()),
        collector_mode: "explicit".to_string(),
        collector_enabled: vec!["test-coll".to_string()],
        collector_disabled: vec![],
        collector_registry: registry,
        nvidia_collector_enabled: false,
        custom_collector_script: None,
        collector_timeout_secs: 5,
        collector_max_output_bytes: 1024 * 1024,
    }
}

fn has_error(errors: &[String], fragment: &str) -> bool {
    errors.iter().any(|e| e.contains(fragment))
}

// ── Tests ──

#[tokio::test]
async fn empty_registry_scripts_not_executed() {
    let tc = tmp_collector("empty-reg", "test-coll", "1.0.0");
    let (dm, mm) = markers(&tc);
    let _ = std::fs::remove_file(&dm);
    let _ = std::fs::remove_file(&mm);

    let (gpus, errors) = gpu::collect_gpus(&make_config(&tc, vec![])).await;
    assert!(gpus.is_empty());
    assert!(!dm.exists());
    assert!(!mm.exists());
    assert!(has_error(&errors, "registry is empty"), "{errors:?}");
    assert!(has_error(&errors, "not registered"), "{errors:?}");
}

#[tokio::test]
async fn wrong_id_version_not_executed() {
    let tc = tmp_collector("wrong-id", "test-coll", "1.0.0");
    let (dm, mm) = markers(&tc);
    let _ = std::fs::remove_file(&dm);
    let _ = std::fs::remove_file(&mm);

    let other = tmp_collector("other", "other-coll", "1.0.0");
    let wrong = registry_entry("other-coll", "1.0.0", &other, true);

    let (gpus, errors) = gpu::collect_gpus(&make_config(&tc, vec![wrong])).await;
    assert!(gpus.is_empty());
    assert!(!dm.exists());
    assert!(!mm.exists());
    assert!(has_error(&errors, "not registered"), "{errors:?}");
}

#[tokio::test]
async fn disabled_in_registry_not_executed() {
    let tc = tmp_collector("disabled", "test-coll", "1.0.0");
    let (dm, mm) = markers(&tc);
    let _ = std::fs::remove_file(&dm);
    let _ = std::fs::remove_file(&mm);

    let entry = registry_entry("test-coll", "1.0.0", &tc, false);
    let (gpus, errors) = gpu::collect_gpus(&make_config(&tc, vec![entry])).await;
    assert!(gpus.is_empty());
    assert!(!dm.exists());
    assert!(!mm.exists());
    assert!(
        has_error(&errors, "disabled in Server registry"),
        "{errors:?}"
    );
}

#[tokio::test]
async fn discover_hash_mismatch_not_executed() {
    let tc = tmp_collector("disc-hash", "test-coll", "1.0.0");
    let (dm, mm) = markers(&tc);
    let _ = std::fs::remove_file(&dm);
    let _ = std::fs::remove_file(&mm);

    let mut entry = registry_entry("test-coll", "1.0.0", &tc, true);
    entry.discover_sha256 =
        "0000000000000000000000000000000000000000000000000000000000000000".to_string();

    let (gpus, errors) = gpu::collect_gpus(&make_config(&tc, vec![entry])).await;
    assert!(gpus.is_empty());
    assert!(!dm.exists());
    assert!(!mm.exists());
    assert!(
        has_error(&errors, "discover.sh hash mismatch"),
        "{errors:?}"
    );
}

#[tokio::test]
async fn metrics_hash_mismatch_not_executed() {
    let tc = tmp_collector("metr-hash", "test-coll", "1.0.0");
    let (dm, mm) = markers(&tc);
    let _ = std::fs::remove_file(&dm);
    let _ = std::fs::remove_file(&mm);

    let mut entry = registry_entry("test-coll", "1.0.0", &tc, true);
    entry.metrics_sha256 =
        "0000000000000000000000000000000000000000000000000000000000000000".to_string();

    let (gpus, errors) = gpu::collect_gpus(&make_config(&tc, vec![entry])).await;
    assert!(gpus.is_empty());
    assert!(!dm.exists());
    assert!(!mm.exists());
    assert!(has_error(&errors, "metrics.sh hash mismatch"), "{errors:?}");
}

#[tokio::test]
async fn hash_match_and_enabled_executes() {
    let tc = tmp_collector("success", "test-coll", "1.0.0");
    let (dm, mm) = markers(&tc);
    let _ = std::fs::remove_file(&dm);
    let _ = std::fs::remove_file(&mm);

    let entry = registry_entry("test-coll", "1.0.0", &tc, true);
    let (_gpus, errors) = gpu::collect_gpus(&make_config(&tc, vec![entry])).await;
    assert!(dm.exists());
    assert!(mm.exists());
    let bad: Vec<_> = errors
        .iter()
        .filter(|e| {
            e.contains("hash mismatch") || e.contains("not registered") || e.contains("disabled")
        })
        .collect();
    assert!(bad.is_empty(), "{errors:?}");
}

#[tokio::test]
async fn configured_root_no_legacy_fallback() {
    let tc = tmp_collector("no-fallback", "test-coll", "1.0.0");
    let (dm, mm) = markers(&tc);
    let _ = std::fs::remove_file(&dm);
    let _ = std::fs::remove_file(&mm);

    let mut config = make_config(&tc, vec![]);
    config.nvidia_collector_enabled = true;

    let (_gpus, errors) = gpu::collect_gpus(&config).await;
    assert!(!dm.exists());
    assert!(!mm.exists());
    assert!(has_error(&errors, "registry is empty"), "{errors:?}");
}

#[tokio::test]
async fn no_collector_root_legacy_works() {
    let config = gpu::CollectorConfig {
        collector_root: None,
        collector_mode: "explicit".to_string(),
        collector_enabled: vec![],
        collector_disabled: vec![],
        collector_registry: vec![],
        nvidia_collector_enabled: true,
        custom_collector_script: None,
        collector_timeout_secs: 5,
        collector_max_output_bytes: 1024 * 1024,
    };
    let (_gpus, errors) = gpu::collect_gpus(&config).await;
    assert!(!has_error(&errors, "registry is empty"), "{errors:?}");
    assert!(!has_error(&errors, "not registered"), "{errors:?}");
}

#[tokio::test]
async fn inspect_does_not_make_trusted() {
    let tc = tmp_collector("inspect", "test-coll", "1.0.0");
    let (dm, mm) = markers(&tc);
    let _ = std::fs::remove_file(&dm);
    let _ = std::fs::remove_file(&mm);

    let output = lightai_agent::collector::inspect::inspect(&tc.dir).unwrap();
    assert_eq!(output.id, "test-coll");
    assert!(!output.discover_sha256.is_empty());
    assert!(!output.metrics_sha256.is_empty());
    assert!(!dm.exists(), "inspect must NOT execute scripts");
    assert!(!mm.exists(), "inspect must NOT execute scripts");

    let (gpus, errors) = gpu::collect_gpus(&make_config(&tc, vec![])).await;
    assert!(gpus.is_empty());
    assert!(!dm.exists());
    assert!(has_error(&errors, "not registered"), "{errors:?}");
}

#[tokio::test]
async fn registry_update_flows_to_next_cycle() {
    let tc = tmp_collector("flow", "test-coll", "1.0.0");
    let (dm, mm) = markers(&tc);
    let _ = std::fs::remove_file(&dm);
    let _ = std::fs::remove_file(&mm);

    let mut config = make_config(&tc, vec![]);
    let (g1, e1) = gpu::collect_gpus(&config).await;
    assert!(g1.is_empty());
    assert!(!dm.exists());
    assert!(has_error(&e1, "not registered"), "{e1:?}");

    // Simulate heartbeat response delivering registry.
    config.collector_registry = vec![registry_entry("test-coll", "1.0.0", &tc, true)];
    let (_g2, e2) = gpu::collect_gpus(&config).await;
    assert!(dm.exists());
    assert!(mm.exists());
    assert!(!has_error(&e2, "not registered"), "{e2:?}");
}
