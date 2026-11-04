//! Codex rollout files exist in two forms: `rollout-*.jsonl` and the zstd
//! compressed `rollout-*.jsonl.zst` that Codex writes for idle sessions
//! (`local_thread_store_compression`). Usage parsing reads both transparently.

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

const COMPRESSED_SUFFIX: &str = ".zst";
const PLAIN_SUFFIX: &str = ".jsonl";
const READER_CAPACITY: usize = 1 << 20;

fn file_name(path: &Path) -> Option<&str> {
    path.file_name().and_then(|name| name.to_str())
}

/// Whether the path is the compressed form (`*.jsonl.zst`).
pub fn is_compressed(path: &Path) -> bool {
    file_name(path).is_some_and(|name| {
        name.strip_suffix(COMPRESSED_SUFFIX)
            .is_some_and(|plain| plain.ends_with(PLAIN_SUFFIX))
    })
}

/// Whether the path is a rollout file (`*.jsonl` or `*.jsonl.zst`), by extension only.
pub fn is_rollout_file(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") || is_compressed(path)
}

/// The plain form of a path (`.zst` removed); plain paths are returned unchanged.
pub fn logical_path(path: &Path) -> PathBuf {
    match file_name(path)
        .filter(|_| is_compressed(path))
        .and_then(|name| name.strip_suffix(COMPRESSED_SUFFIX))
    {
        Some(plain) => path.with_file_name(plain),
        None => path.to_path_buf(),
    }
}

/// Line reader that decompresses `.zst` files transparently.
pub fn reader_from_file(path: &Path, file: File) -> io::Result<Box<dyn BufRead + Send>> {
    if is_compressed(path) {
        let decoder = zstd::stream::read::Decoder::new(file)?;
        Ok(Box::new(BufReader::with_capacity(READER_CAPACITY, decoder)))
    } else {
        Ok(Box::new(BufReader::with_capacity(READER_CAPACITY, file)))
    }
}

/// While Codex compresses or decompresses a rollout both forms can exist for a
/// moment. Keep only the plain one: it is the file Codex writes to and holds at
/// least as much content. Order is preserved.
pub fn dedupe_siblings(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let plain: HashSet<PathBuf> = paths
        .iter()
        .filter(|path| !is_compressed(path))
        .cloned()
        .collect();
    paths
        .into_iter()
        .filter(|path| !is_compressed(path) || !plain.contains(&logical_path(path)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_both_forms() {
        let plain = Path::new("/x/rollout-a.jsonl");
        let zst = Path::new("/x/rollout-a.jsonl.zst");
        assert!(is_rollout_file(plain));
        assert!(is_rollout_file(zst));
        assert!(!is_compressed(plain));
        assert!(is_compressed(zst));
        assert!(!is_rollout_file(Path::new("/x/rollout-a.zst")));
        assert!(!is_rollout_file(Path::new("/x/notes.txt")));
        assert_eq!(logical_path(zst), plain);
        assert_eq!(logical_path(plain), plain);
    }

    #[test]
    fn dedupe_keeps_plain_and_order() {
        let paths = vec![
            PathBuf::from("/x/b.jsonl.zst"),
            PathBuf::from("/x/a.jsonl.zst"),
            PathBuf::from("/x/a.jsonl"),
            PathBuf::from("/y/a.jsonl.zst"),
        ];
        assert_eq!(
            dedupe_siblings(paths),
            vec![
                PathBuf::from("/x/b.jsonl.zst"),
                PathBuf::from("/x/a.jsonl"),
                PathBuf::from("/y/a.jsonl.zst"),
            ]
        );
    }
}
