use std::{env, fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("Cargo manifest directory");
    let root = Path::new(&manifest).parent().unwrap().parent().unwrap();
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", root.join(".git").display());
    let (revision, tag, dirty) = if root.join(".git").exists() {
        let top = git(root, &["rev-parse", "--show-toplevel"]).expect(
            "Git metadata exists but Git cannot read it; install Git or repair the checkout",
        );
        assert_eq!(
            fs::canonicalize(top).unwrap(),
            fs::canonicalize(root).unwrap(),
            "Git must identify the Core checkout"
        );
        let revision = git(root, &["rev-parse", "--verify", "HEAD"])
            .expect("Core checkout must have a commit");
        let tag = git(root, &["describe", "--tags", "--exact-match", "HEAD"]).unwrap_or_default();
        let dirty = !git(root, &["status", "--porcelain", "--untracked-files=no"])
            .expect("Cannot inspect Core checkout")
            .is_empty();
        // Watch tracked sources as well as Git refs: committing without changing Rust
        // sources must invalidate Cargo's embedded build identity, including worktrees.
        for name in ["HEAD", "index", "packed-refs", "refs"] {
            if let Some(file) = git(root, &["rev-parse", "--git-path", name]) {
                println!("cargo:rerun-if-changed={}", root.join(file).display());
            }
        }
        if let Some(reference) = git(root, &["symbolic-ref", "-q", "HEAD"])
            && let Some(file) = git(root, &["rev-parse", "--git-path", &reference])
        {
            println!("cargo:rerun-if-changed={}", root.join(file).display());
        }
        let files = git(root, &["ls-files", "-z"]).expect("Cannot inspect tracked Core files");
        for file in files.split('\0').filter(|file| !file.is_empty()) {
            println!("cargo:rerun-if-changed={}", root.join(file).display());
        }
        (revision, tag, dirty.to_string())
    } else {
        (String::new(), String::new(), String::new())
    };
    println!("cargo:rustc-env=MOMO_GIT_REVISION={revision}");
    println!("cargo:rustc-env=MOMO_GIT_TAG={tag}");
    println!("cargo:rustc-env=MOMO_GIT_DIRTY={dirty}");
}
