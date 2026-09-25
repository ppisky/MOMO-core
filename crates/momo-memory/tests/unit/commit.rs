use super::*;

fn fixture() -> (tempfile::TempDir, MemoryWorkspace, PreparedMemoryCommit) {
    let directory = tempfile::tempdir().expect("directory");
    let workspace = MemoryWorkspace::initialize(directory.path()).expect("workspace");
    let plan = PreparedMemoryCommit::prepare(
        workspace.root(),
        &[
            FileMutation::Write {
                path: workspace.root().join("events/a.md"),
                content: b"first".to_vec(),
            },
            FileMutation::Write {
                path: workspace.root().join("events/b.md"),
                content: b"second".to_vec(),
            },
        ],
    )
    .expect("plan");
    (directory, workspace, plan)
}

#[test]
fn durable_plan_recovers_partial_write_and_replays_exactly() {
    let (directory, workspace, plan) = fixture();
    let durable = serde_json::to_string(&plan).expect("persist plan");
    fs::write(workspace.root().join("events/a.md"), b"first").expect("partial write");
    drop(workspace);
    let reopened = MemoryWorkspace::initialize(directory.path()).expect("reopen");
    let plan = serde_json::from_str(&durable).expect("load plan");
    reopened.apply_prepared_commit(&plan).expect("recover");
    reopened.apply_prepared_commit(&plan).expect("replay");
    assert_eq!(
        fs::read(reopened.root().join("events/b.md")).expect("second file"),
        b"second"
    );
}

#[test]
fn conflicts_and_unsafe_targets_fail_before_any_file_write() {
    let (_directory, workspace, mut plan) = fixture();
    fs::write(workspace.root().join("events/b.md"), b"operator edit").expect("edit");
    assert!(workspace.apply_prepared_commit(&plan).is_err());
    assert!(!workspace.root().join("events/a.md").exists());
    plan.files[1].path = "../outside".to_owned();
    assert!(workspace.apply_prepared_commit(&plan).is_err());
    assert!(!workspace.root().join("events/a.md").exists());
}
