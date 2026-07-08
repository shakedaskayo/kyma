use crate::registry::*;

#[test]
fn layout_derived_from_selector_shape() {
    assert_eq!(RealmSelector::Realms(vec!["kyma".into()]).layout(), VaultLayout::Flat);
    assert_eq!(
        RealmSelector::Realms(vec!["a".into(), "b".into()]).layout(),
        VaultLayout::ByRealm
    );
    assert_eq!(RealmSelector::All.layout(), VaultLayout::ByRealm);
}

#[test]
fn config_new_validates_name_and_realms() {
    let now = "2026-07-08T00:00:00Z";
    assert!(BrainConfig::new("team-brain", RealmSelector::All, now).is_ok());
    assert!(BrainConfig::new("Team", RealmSelector::All, now).is_err());
    assert!(BrainConfig::new("-x", RealmSelector::All, now).is_err());
    assert!(BrainConfig::new("t", RealmSelector::Realms(vec![]), now).is_err());
    assert!(BrainConfig::new("t", RealmSelector::Realms(vec!["a b".into()]), now).is_err());
}

#[test]
fn run_ring_caps() {
    let mut rt = BrainRuntime::default();
    for i in 0..60 {
        rt.record_run(BrainRunRecord {
            kind: "export".into(),
            started_at: format!("t{i}"),
            finished_at: format!("t{i}"),
            commit: None,
            files_written: 0,
            files_deleted: 0,
            notes_ingested: 0,
            noop: true,
            error: None,
            warnings: vec![],
        });
    }
    assert_eq!(rt.runs.len(), RUN_RING);
    assert_eq!(rt.runs[0].started_at, "t59");
}

#[test]
fn config_serde_round_trips() {
    let cfg = BrainConfig::new(
        "team",
        RealmSelector::Realms(vec!["kyma".into(), "global".into()]),
        "2026-07-08T00:00:00Z",
    )
    .unwrap();
    let json = serde_json::to_string(&cfg).unwrap();
    let back: BrainConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(cfg, back);
}
