#[test]
fn installer_smoke_precedes_release_publication() {
    let workflow = include_str!("../.github/workflows/release.yml");
    let smoke = workflow.find("- name: Smoke-test").unwrap();
    let publish = workflow.find("- name: Create version release").unwrap();

    assert!(smoke < publish);
}
