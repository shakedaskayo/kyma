use crate::worker::{launchd_plist, plist_path, sync_args, systemd_unit, unit_path, WorkerOptions};

#[test]
fn sync_args_compose_from_options() {
    assert_eq!(sync_args(&WorkerOptions::default()), vec!["sync", "--watch"]);
    assert_eq!(
        sync_args(&WorkerOptions {
            cc_only: true,
            ..WorkerOptions::default()
        }),
        vec!["sync", "--watch", "--cc-only"]
    );
    assert_eq!(
        sync_args(&WorkerOptions {
            cloud_only: true,
            ..WorkerOptions::default()
        }),
        vec!["sync", "--watch", "--cloud-only"]
    );
}

#[test]
fn launchd_plist_renders_service() {
    let opts = WorkerOptions {
        interval_secs: Some(120),
        cc_only: true,
        cloud_only: false,
        pensieve_home: Some("/Users/x/.pensieve".to_string()),
    };
    let plist = launchd_plist("/usr/local/bin/pensieve", &opts, "/Users/x/.pensieve/logs/worker.log");
    assert!(plist.contains("<string>dev.getpensieve.pensieve-sync</string>"));
    assert!(plist.contains("<string>/usr/local/bin/pensieve</string>"));
    assert!(plist.contains("<string>sync</string>"));
    assert!(plist.contains("<string>--watch</string>"));
    assert!(plist.contains("<string>--cc-only</string>"));
    assert!(plist.contains("<key>KeepAlive</key>"));
    assert!(plist.contains("<string>/Users/x/.pensieve/logs/worker.log</string>"));
    assert!(plist.contains("<key>PENSIEVE_CC_SYNC_POLL_SECS</key>"));
    assert!(plist.contains("<string>120</string>"));
    // Services don't inherit the shell env — the store location is pinned.
    assert!(plist.contains("<key>PENSIEVE_HOME</key>"));
    assert!(plist.contains("<string>/Users/x/.pensieve</string>"));
    // Working dir is the store: relative caches (fastembed models) land in a
    // stable, writable place instead of launchd's default `/`.
    assert!(plist.contains("<key>WorkingDirectory</key>"));

    // No interval / no home → no env block.
    let plain = launchd_plist("/usr/local/bin/pensieve", &WorkerOptions::default(), "/tmp/w.log");
    assert!(!plain.contains("EnvironmentVariables"));
    assert!(!plain.contains("--cc-only"));
}

#[test]
fn systemd_unit_renders_service() {
    let opts = WorkerOptions {
        interval_secs: Some(60),
        pensieve_home: Some("/home/x/.pensieve".to_string()),
        ..WorkerOptions::default()
    };
    let unit = systemd_unit("/usr/bin/pensieve", &opts);
    assert!(unit.contains("ExecStart=/usr/bin/pensieve sync --watch"));
    assert!(unit.contains("Restart=on-failure"));
    assert!(unit.contains("Environment=PENSIEVE_CC_SYNC_POLL_SECS=60"));
    assert!(unit.contains("Environment=PENSIEVE_HOME=/home/x/.pensieve"));
    assert!(unit.contains("WorkingDirectory=/home/x/.pensieve"));
    assert!(unit.contains("WantedBy=default.target"));

    let plain = systemd_unit("/usr/bin/pensieve", &WorkerOptions::default());
    assert!(!plain.contains("Environment="));
}

#[test]
fn service_paths_live_under_home() {
    assert_eq!(
        plist_path("/Users/x").display().to_string(),
        "/Users/x/Library/LaunchAgents/dev.getpensieve.pensieve-sync.plist"
    );
    assert_eq!(
        unit_path("/home/x").display().to_string(),
        "/home/x/.config/systemd/user/pensieve-sync.service"
    );
}
