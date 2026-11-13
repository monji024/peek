// https://github.com/monji024

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

const MAX_SNAPSHOT_FILES: usize = 2_000;
const MAX_HASH_FILE_SIZE: u64 = 512 * 1024;
const MAX_TREE_DEPTH: usize = 8;

const EXCLUDED_NAMES: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".ssh",
    ".gnupg",
    ".aws",
    ".env",
    "id_rsa",
    "id_ed25519",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntryMeta {
    pub rel_path: PathBuf,
    pub size: u64,
    pub mtime_sec: u64,
    pub mode: u32,
    pub is_symlink: bool,
    pub content_hash: u64,
}

#[derive(Debug, Clone)]
pub struct FsSnapshot {
    pub id: usize,
    pub timestamp_ms: u64,
    pub pid: u32,
    pub step_index: usize,
    pub root_scope: PathBuf,
    pub entries: BTreeMap<PathBuf, FileEntryMeta>,
    pub skipped_errors: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffKind {
    Added,
    Modified,
    Deleted,
}

impl DiffKind {
    pub fn symbol(&self) -> &'static str {
        match self {
            DiffKind::Added => "+",
            DiffKind::Modified => "~",
            DiffKind::Deleted => "-",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileDiffEntry {
    pub rel_path: PathBuf,
    pub kind: DiffKind,
    pub old_size: Option<u64>,
    pub new_size: Option<u64>,
    pub mode: u32,
}

#[derive(Debug, Clone)]
pub struct SnapshotDiff {
    pub from_id: usize,
    pub to_id: usize,
    pub pid: u32,
    pub step_index: usize,
    pub changes: Vec<FileDiffEntry>,
}

impl FsSnapshot {
    pub fn capture(id: usize, pid: u32, step_index: usize, scope: &Path) -> Result<Self> {
        let canonical_scope = fs::canonicalize(scope)
            .with_context(|| format!("failed to canonicalize snapshot scope: {:?}", scope))?;

        if canonical_scope == Path::new("/") {
            bail!("refusing to snapshot root filesystem '/'");
        }

        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let mut entries = BTreeMap::new();
        let mut skipped_errors = 0usize;
        let mut truncated = false;

        if canonical_scope.is_file() {
            if let Some(meta) = inspect_single_path(
                &canonical_scope,
                canonical_scope.parent().unwrap_or(&canonical_scope),
            ) {
                entries.insert(meta.rel_path.clone(), meta);
            }
        } else {
            walk_bounded_dir(
                &canonical_scope,
                &canonical_scope,
                0,
                &mut entries,
                &mut skipped_errors,
                &mut truncated,
            );
        }

        Ok(Self {
            id,
            timestamp_ms,
            pid,
            step_index,
            root_scope: canonical_scope,
            entries,
            skipped_errors,
            truncated,
        })
    }

    pub fn compare(older: &FsSnapshot, newer: &FsSnapshot) -> SnapshotDiff {
        let mut changes = Vec::new();

        for (path, new_meta) in &newer.entries {
            match older.entries.get(path) {
                None => {
                    changes.push(FileDiffEntry {
                        rel_path: path.clone(),
                        kind: DiffKind::Added,
                        old_size: None,
                        new_size: Some(new_meta.size),
                        mode: new_meta.mode,
                    });
                }
                Some(old_meta) => {
                    if old_meta.size != new_meta.size
                        || old_meta.content_hash != new_meta.content_hash
                        || old_meta.mode != new_meta.mode
                        || old_meta.is_symlink != new_meta.is_symlink
                        || (old_meta.mtime_sec != new_meta.mtime_sec
                            && new_meta.size > MAX_HASH_FILE_SIZE)
                    {
                        changes.push(FileDiffEntry {
                            rel_path: path.clone(),
                            kind: DiffKind::Modified,
                            old_size: Some(old_meta.size),
                            new_size: Some(new_meta.size),
                            mode: new_meta.mode,
                        });
                    }
                }
            }
        }

        for (path, old_meta) in &older.entries {
            if !newer.entries.contains_key(path) {
                changes.push(FileDiffEntry {
                    rel_path: path.clone(),
                    kind: DiffKind::Deleted,
                    old_size: Some(old_meta.size),
                    new_size: None,
                    mode: old_meta.mode,
                });
            }
        }

        SnapshotDiff {
            from_id: older.id,
            to_id: newer.id,
            pid: newer.pid,
            step_index: newer.step_index,
            changes,
        }
    }
}

fn is_excluded_name(name: &str) -> bool {
    if EXCLUDED_NAMES.contains(&name) {
        return true;
    }
    name.starts_with(".env.")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.ends_with(".p12")
}

fn walk_bounded_dir(
    root: &Path,
    current: &Path,
    depth: usize,
    entries: &mut BTreeMap<PathBuf, FileEntryMeta>,
    skipped_errors: &mut usize,
    truncated: &mut bool,
) {
    if depth > MAX_TREE_DEPTH || entries.len() >= MAX_SNAPSHOT_FILES {
        if entries.len() >= MAX_SNAPSHOT_FILES {
            *truncated = true;
        }
        return;
    }

    let read_dir = match fs::read_dir(current) {
        Ok(rd) => rd,
        Err(_) => {
            *skipped_errors += 1;
            return;
        }
    };

    for item in read_dir {
        if entries.len() >= MAX_SNAPSHOT_FILES {
            *truncated = true;
            return;
        }
        let Ok(entry) = item else {
            *skipped_errors += 1;
            continue;
        };
        let path = entry.path();
        let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if is_excluded_name(fname) {
            continue;
        }

        let Ok(sym_meta) = fs::symlink_metadata(&path) else {
            *skipped_errors += 1;
            continue;
        };

        let ftype = sym_meta.file_type();
        if ftype.is_symlink() {
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            let target_hash = fs::read_link(&path)
                .map(|t| fnv1a_hash(t.to_string_lossy().as_bytes()))
                .unwrap_or(0);
            entries.insert(
                rel.clone(),
                FileEntryMeta {
                    rel_path: rel,
                    size: sym_meta.len(),
                    mtime_sec: mtime_seconds(&sym_meta),
                    mode: sym_meta.permissions().mode() & 0o777,
                    is_symlink: true,
                    content_hash: target_hash,
                },
            );
            continue;
        }

        if ftype.is_dir() {
            walk_bounded_dir(root, &path, depth + 1, entries, skipped_errors, truncated);
        } else if ftype.is_file() {
            if let Some(meta) = inspect_single_path(&path, root) {
                entries.insert(meta.rel_path.clone(), meta);
            } else {
                *skipped_errors += 1;
            }
        }
    }
}

fn inspect_single_path(path: &Path, root: &Path) -> Option<FileEntryMeta> {
    let sym_meta = fs::symlink_metadata(path).ok()?;
    let rel = path.strip_prefix(root).unwrap_or(path).to_path_buf();
    let size = sym_meta.len();
    let mtime_sec = mtime_seconds(&sym_meta);
    let mode = sym_meta.permissions().mode() & 0o777;

    let content_hash = if size <= MAX_HASH_FILE_SIZE {
        let mut f = fs::File::open(path).ok()?;
        let mut buf = Vec::with_capacity(size as usize);
        f.read_to_end(&mut buf).ok()?;
        fnv1a_hash(&buf)
    } else {
        fnv1a_hash(&size.to_le_bytes()) ^ fnv1a_hash(&mtime_sec.to_le_bytes())
    };

    Some(FileEntryMeta {
        rel_path: rel,
        size,
        mtime_sec,
        mode,
        is_symlink: false,
        content_hash,
    })
}

fn mtime_seconds(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn fnv1a_hash(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_and_diffs_directory_snapshots() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f1 = dir.path().join("alpha.txt");
        let f2 = dir.path().join("beta.txt");
        let secret = dir.path().join(".env");

        fs::write(&f1, "hello v1").unwrap();
        fs::write(&f2, "to be deleted").unwrap();
        fs::write(&secret, "SECRET_KEY=123").unwrap();

        let snap1 = FsSnapshot::capture(1, 100, 1, dir.path()).expect("snap1");
        assert!(snap1.entries.contains_key(Path::new("alpha.txt")));
        assert!(snap1.entries.contains_key(Path::new("beta.txt")));
        assert!(!snap1.entries.contains_key(Path::new(".env")));

        fs::write(&f1, "hello v2 modified").unwrap();
        fs::remove_file(&f2).unwrap();
        let f3 = dir.path().join("gamma.txt");
        fs::write(&f3, "newly created").unwrap();

        let snap2 = FsSnapshot::capture(2, 100, 2, dir.path()).expect("snap2");
        let diff = FsSnapshot::compare(&snap1, &snap2);

        assert_eq!(diff.changes.len(), 3);
        assert!(diff
            .changes
            .iter()
            .any(|c| c.rel_path == Path::new("alpha.txt") && c.kind == DiffKind::Modified));
        assert!(diff
            .changes
            .iter()
            .any(|c| c.rel_path == Path::new("beta.txt") && c.kind == DiffKind::Deleted));
        assert!(diff
            .changes
            .iter()
            .any(|c| c.rel_path == Path::new("gamma.txt") && c.kind == DiffKind::Added));
    }
}
