use kyma_datasources::secrets::{EnvSecretStore, SecretError, SecretStore};

#[test]
fn literal_passes_through() {
    let s = EnvSecretStore;
    assert_eq!(s.resolve("hunter2").unwrap(), "hunter2");
}

#[test]
fn env_reference_resolves() {
    std::env::set_var("KYMA_CONN_TEST_SECRET_42", "sekret");
    let s = EnvSecretStore;
    assert_eq!(
        s.resolve("$env:KYMA_CONN_TEST_SECRET_42").unwrap(),
        "sekret"
    );
    std::env::remove_var("KYMA_CONN_TEST_SECRET_42");
}

#[test]
fn env_reference_missing_yields_not_found() {
    std::env::remove_var("KYMA_CONN_TEST_MISSING_99");
    let s = EnvSecretStore;
    match s.resolve("$env:KYMA_CONN_TEST_MISSING_99") {
        Err(SecretError::NotFound(name)) => {
            assert_eq!(name, "KYMA_CONN_TEST_MISSING_99")
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
}
