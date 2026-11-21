//! Distributed compilation cluster orchestration and worker node subsystems for DBS.
//!
//! Provides a scalable, pull-based architecture using PostgreSQL as a centralized,
//! lock-free job queue (`SELECT ... FOR UPDATE SKIP LOCKED`), Mock for isolated
//! chroot compilation, dynamic remote repository feedback, and live telemetry tracking.

pub mod coordinator;
pub mod worker;

pub use coordinator::{
    coordinate_layer_build, prepare_layer_srpms, spawn_coordinator_http, CoordinatorConfig,
};
pub use worker::{run_worker, WorkerConfig};

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_worker_config_defaults() {
        let cfg = WorkerConfig::default();
        assert_eq!(cfg.concurrency, 4);
        assert_eq!(cfg.heartbeat_interval_secs, 10);
        assert_eq!(cfg.poll_interval_secs, 3);
        assert!(!cfg.keep_build_dir);
        assert!(!cfg.nocheck);
        assert!(cfg.worker_id >= 100);
        assert!(!cfg.node_name.is_empty());
    }

    #[test]
    fn test_coordinator_config_initialization() {
        let temp = tempdir().unwrap();
        let staging = temp.path().join("staging");

        let cfg = CoordinatorConfig {
            host: "127.0.0.1".to_string(),
            port: 8080,
            public_url: "http://127.0.0.1:8080".to_string(),
            staging_dir: staging.clone(),
            distro_name: "tacos-stable-x86_64".to_string(),
            arch: "x86_64".to_string(),
            mock_root: "tacos-stable-x86_64".to_string(),
            mock_config_dir: None,
            lookaside_dir: None,
            local_workers: 4,
            db_url: None,
        };

        assert_eq!(cfg.port, 8080);
        assert_eq!(cfg.local_workers, 4);
        assert_eq!(cfg.distro_name, "tacos-stable-x86_64");
    }

    #[tokio::test]
    async fn test_embedded_http_server_upload_and_download() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();

        // Bind on test port
        let port: u16 = 18987;
        let root_clone = root.clone();
        tokio::spawn(async move {
            let _ = crate::distro::run_http_server(root_clone, "127.0.0.1", port).await;
        });

        // Give the server a moment to bind
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let client = reqwest::Client::new();
        let base_url = format!("http://127.0.0.1:{}", port);

        // 1. Test health check endpoint
        let health_resp = client.get(&format!("{}/health", base_url)).send().await.unwrap();
        assert!(health_resp.status().is_success());
        let health_json: serde_json::Value = health_resp.json().await.unwrap();
        assert_eq!(health_json["status"], "ok");

        // 2. Test PUT file upload
        let test_payload = b"test-rpm-binary-content-12345";
        let upload_url = format!("{}/upload/staging/rpms/x86_64/test-pkg-1.0-1.tcrs.x86_64.rpm", base_url);
        let put_resp = client.put(&upload_url)
            .body(test_payload.to_vec())
            .send()
            .await
            .unwrap();
        assert!(put_resp.status().is_success());

        // 3. Verify file was created on disk in correct path
        let on_disk = root.join("staging/rpms/x86_64/test-pkg-1.0-1.tcrs.x86_64.rpm");
        assert!(on_disk.is_file());
        let read_disk = fs::read(&on_disk).unwrap();
        assert_eq!(read_disk, test_payload);

        // 4. Test GET file download over HTTP
        let download_url = format!("{}/staging/rpms/x86_64/test-pkg-1.0-1.tcrs.x86_64.rpm", base_url);
        let get_resp = client.get(&download_url).send().await.unwrap();
        assert!(get_resp.status().is_success());
        let get_bytes = get_resp.bytes().await.unwrap();
        assert_eq!(get_bytes.as_ref(), test_payload);
    }
}

