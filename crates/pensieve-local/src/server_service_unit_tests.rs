use crate::server_service::{
    launchd_plist, plist_path, serve_args, systemd_unit, unit_path, ServerOptions,
};

#[test]
fn serve_args_compose_from_options() {
    assert_eq!(
        serve_args(&ServerOptions {
            addr: "127.0.0.1:7777".into(),
            ..ServerOptions::default()
        }),
        vec!["serve", "--addr", "127.0.0.1:7777"]
    );
}

#[test]
fn launchd_plist_keeps_the_server_alive_across_crash_and_boot() {
    let opts = ServerOptions {
        addr: "127.0.0.1:7777".into(),
        token: Some("tok123".into()),
        pensieve_home: Some("/Users/x/.pensieve".to_string()),
        secret_key: Some("c2VjcmV0LWtleS10ZXN0".into()),
    };
    let plist = launchd_plist("/usr/local/bin/pensieve", &opts, "/Users/x/.pensieve/logs/server.log");
    assert!(plist.contains("<string>dev.getpensieve.pensieve-server</string>"));
    assert!(plist.contains("<string>/usr/local/bin/pensieve</string>"));
    assert!(plist.contains("<string>serve</string>"));
    assert!(plist.contains("<string>--addr</string>"));
    assert!(plist.contains("<string>127.0.0.1:7777</string>"));
    // The whole point: restart on crash, start at login.
    assert!(plist.contains("<key>KeepAlive</key>"));
    assert!(plist.contains("<key>RunAtLoad</key>"));
    assert!(plist.contains("<string>/Users/x/.pensieve/logs/server.log</string>"));
    // Auth token rides the service env (services don't inherit the shell's).
    assert!(plist.contains("<key>PENSIEVE_AUTH_TOKENS</key>"));
    assert!(plist.contains("<string>tok123:admin</string>"));
    assert!(plist.contains("<key>PENSIEVE_HOME</key>"));
    assert!(plist.contains("<key>WorkingDirectory</key>"));
    // Credential-encryption key rides the service env too — the server can't
    // decrypt stored data source credentials without it.
    assert!(plist.contains("<key>PENSIEVE_SECRET_KEY</key>"));
    assert!(plist.contains("<string>c2VjcmV0LWtleS10ZXN0</string>"));

    // No token → no PENSIEVE_AUTH_TOKENS entry (auth-disabled local default).
    let plain = launchd_plist(
        "/usr/local/bin/pensieve",
        &ServerOptions {
            addr: "127.0.0.1:7777".into(),
            ..ServerOptions::default()
        },
        "/tmp/s.log",
    );
    assert!(!plain.contains("PENSIEVE_AUTH_TOKENS"));
}

#[test]
fn systemd_unit_restarts_on_failure() {
    let opts = ServerOptions {
        addr: "0.0.0.0:7777".into(),
        token: Some("tok".into()),
        pensieve_home: Some("/home/x/.pensieve".to_string()),
        secret_key: Some("c2VjcmV0".into()),
    };
    let unit = systemd_unit("/usr/bin/pensieve", &opts);
    assert!(unit.contains("ExecStart=/usr/bin/pensieve serve --addr 0.0.0.0:7777"));
    assert!(unit.contains("Restart=on-failure"));
    assert!(unit.contains("Environment=PENSIEVE_AUTH_TOKENS=tok:admin"));
    assert!(unit.contains("Environment=PENSIEVE_HOME=/home/x/.pensieve"));
    assert!(unit.contains("Environment=PENSIEVE_SECRET_KEY=c2VjcmV0"));
    assert!(unit.contains("WantedBy=default.target"));
}

#[test]
fn service_file_paths_are_user_scoped() {
    assert_eq!(
        plist_path("/Users/x").to_string_lossy(),
        "/Users/x/Library/LaunchAgents/dev.getpensieve.pensieve-server.plist"
    );
    assert_eq!(
        unit_path("/home/x").to_string_lossy(),
        "/home/x/.config/systemd/user/pensieve-server.service"
    );
}
