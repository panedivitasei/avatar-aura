// Port of the Mode 1 package walk in avatar-aura/native/avatarextract/main.cpp, run against a real package.
//! Reads the package named by `AVATAR_AURA_STFS_SAMPLE` and skips when it is unset or missing.

#[test]
fn real_package_reads_every_file() {
    let Some(path) = std::env::var_os("AVATAR_AURA_STFS_SAMPLE") else {
        eprintln!("AVATAR_AURA_STFS_SAMPLE unset, skipping");
        return;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("{} unreadable, skipping", path.to_string_lossy());
        return;
    };
    let pkg = stfs::Package::parse(&bytes).expect("parse package");
    assert!(!pkg.entries().is_empty(), "empty file table");
    for e in pkg.entries().iter().filter(|e| !e.is_directory) {
        let data = pkg.read(e).unwrap_or_else(|err| panic!("{}: {err}", pkg.path(e)));
        assert_eq!(data.len(), e.size as usize, "{}", pkg.path(e));
    }
}
