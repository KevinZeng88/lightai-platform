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
async fn no_collector_root_executes_no_collectors() {
    let config = gpu::CollectorConfig {
        collector_root: None,
        collector_mode: "explicit".to_string(),
        collector_enabled: vec![],
        collector_disabled: vec![],
        collector_registry: vec![],
        collector_timeout_secs: 5,
        collector_max_output_bytes: 1024 * 1024,
    };
    let (gpus, errors) = gpu::collect_gpus(&config).await;
    assert!(gpus.is_empty());
    assert!(
        has_error(&errors, "collector root is not configured"),
        "{errors:?}"
    );
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

/// When nvidia-smi is at an uncommon absolute path, the collector script
/// should find it via the absolute-path search and execute it successfully.
#[tokio::test]
async fn script_finds_tool_via_absolute_path_fallback() {
    let tc = tmp_collector("abs-path", "test-coll", "1.0.0");
    let fake_bin = tc.dir.join("fake-smi");

    // Create a fake nvidia-smi that just outputs the expected TSV.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(
            &fake_bin,
            "#!/bin/sh\necho '0, FakeGPU, GPU-000, 0000:01:00.0, 550.54'\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // Write a discover script that searches absolute paths (like the real NVIDIA one).
    let ds = format!(
        r#"#!/bin/sh
set -e
export PATH="/usr/bin:/bin"
NVIDIA_SMI_BIN=""
for c in /no/such/smi /tmp/no/smi "{}" /usr/bin/nvidia-smi; do
    if [ -x "$c" ]; then NVIDIA_SMI_BIN="$c"; break; fi
done
if [ -z "$NVIDIA_SMI_BIN" ]; then
    printf 'STATUS\t1\tnot_available\ttest\ttest-coll\ttool not found; checked paths; PATH=%s\n' "$PATH" >&2
    exit 0
fi
printf 'STATUS\t1\tok\ttest\ttest-coll\t\n'
"$NVIDIA_SMI_BIN" --query-gpu=index,name,uuid,pci.bus_id,driver_version --format=csv,noheader,nounits 2>/dev/null \
    | while IFS=',' read -r idx name uuid pci driver; do
    idx=$(echo "$idx" | tr -d ' ')
    name=$(echo "$name" | tr -d ' ')
    uuid=$(echo "$uuid" | tr -d ' ')
    printf 'DEVICE\t1\ttest:GPU-000\ttest\t0\t%s\t%s\t%s\t\n' "$name" "$uuid" "$driver"
done
"#,
        fake_bin.display()
    );
    let dp = tc.dir.join("discover.sh");
    std::fs::write(&dp, ds).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dp, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // metrics.sh: same lookup, just outputs METRIC.
    let ms = format!(
        r#"#!/bin/sh
set -e
export PATH="/usr/bin:/bin"
NVIDIA_SMI_BIN=""
for c in /no/such/smi /tmp/no/smi "{}" /usr/bin/nvidia-smi; do
    if [ -x "$c" ]; then NVIDIA_SMI_BIN="$c"; break; fi
done
if [ -z "$NVIDIA_SMI_BIN" ]; then
    printf 'STATUS\t1\tnot_available\ttest\ttest-coll\ttool not found; checked paths; PATH=%s\n' "$PATH" >&2
    exit 0
fi
printf 'STATUS\t1\tok\ttest\ttest-coll\t\n'
"$NVIDIA_SMI_BIN" --query-gpu=uuid,memory.total,memory.used --format=csv,noheader,nounits 2>/dev/null \
    | while IFS=',' read -r uuid memt memu; do
    uuid=$(echo "$uuid" | tr -d ' ')
    printf 'METRIC\t1\ttest:GPU-000\t100\t50\t50\t0\t0\t0\t0\tok\t\n'
done
"#,
        fake_bin.display()
    );
    let mp = tc.dir.join("metrics.sh");
    std::fs::write(&mp, ms).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&mp, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let config = make_config(&tc, vec![registry_entry("test-coll", "1.0.0", &tc, true)]);
    let (gpus, errors) = gpu::collect_gpus(&config).await;
    assert!(
        !has_error(&errors, "not found"),
        "should have found fake-smi via abs path: {errors:?}"
    );
    assert_eq!(gpus.len(), 1);
    assert_eq!(gpus[0].gpu_key, "test:GPU-000");
}
