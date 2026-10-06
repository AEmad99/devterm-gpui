//! Shared sorting and filtering for the file views.
//! Dirs-first grouping, per-key ordering, deterministic name tie-breaks,
//! and substring filtering.

#![allow(dead_code)]

use std::borrow::Cow;
use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: i64,
    pub mtime_ms: i64,
    pub mode: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileSortKey {
    Name,
    Size,
    Modified,
    Type,
}

impl FileSortKey {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Size => "size",
            Self::Modified => "modified",
            Self::Type => "type",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Size => "Size",
            Self::Modified => "Modified",
            Self::Type => "Type",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileSortDir {
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileSortPrefs {
    pub key: FileSortKey,
    pub dir: FileSortDir,
    pub dirs_first: bool,
}

pub const DEFAULT_FILE_SORT: FileSortPrefs = FileSortPrefs {
    key: FileSortKey::Name,
    dir: FileSortDir::Asc,
    dirs_first: true,
};

pub const FILE_SORT_KEYS: [FileSortKey; 4] = [
    FileSortKey::Name,
    FileSortKey::Size,
    FileSortKey::Modified,
    FileSortKey::Type,
];

/// Lower-cased extension without the dot; empty for extensionless names and dotfiles.
pub fn file_type_key(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

fn compare_names(a: &str, b: &str) -> Ordering {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let mut i = 0;
    let mut j = 0;
    while i < ac.len() && j < bc.len() {
        if ac[i].is_ascii_digit() && bc[j].is_ascii_digit() {
            let mut i2 = i;
            while i2 < ac.len() && ac[i2].is_ascii_digit() {
                i2 += 1;
            }
            let mut j2 = j;
            while j2 < bc.len() && bc[j2].is_ascii_digit() {
                j2 += 1;
            }
            let na: u128 = ac[i..i2].iter().collect::<String>().parse().unwrap_or(0);
            let nb: u128 = bc[j..j2].iter().collect::<String>().parse().unwrap_or(0);
            if na != nb {
                return na.cmp(&nb);
            }
            i = i2;
            j = j2;
            continue;
        }
        let ca = ac[i].to_ascii_lowercase();
        let cb = bc[j].to_ascii_lowercase();
        if ca != cb {
            return ca.cmp(&cb);
        }
        i += 1;
        j += 1;
    }
    ac.len().cmp(&bc.len())
}

/// Return a sorted copy of `entries`. Ties on size/modified/type fall back to
/// name. The direction applies to the whole comparison, including the name tie-break.
/// Directories stay grouped first when `dirs_first` is set.
pub fn sort_file_entries(entries: &[FileEntry], prefs: FileSortPrefs) -> Vec<FileEntry> {
    let mut out = entries.to_vec();
    out.sort_by(|a, b| {
        if prefs.dirs_first && a.is_dir != b.is_dir {
            return if a.is_dir {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let mut c = match prefs.key {
            FileSortKey::Size => a.size.cmp(&b.size),
            FileSortKey::Modified => a.mtime_ms.cmp(&b.mtime_ms),
            FileSortKey::Type => compare_names(&file_type_key(&a.name), &file_type_key(&b.name)),
            FileSortKey::Name => compare_names(&a.name, &b.name),
        };
        if c == Ordering::Equal && prefs.key != FileSortKey::Name {
            c = compare_names(&a.name, &b.name);
        }
        if prefs.dir == FileSortDir::Desc {
            c = c.reverse();
        }
        c
    });
    out
}

/// Keep entries whose name contains `query` (case-insensitive substring).
/// A blank query returns the input slice untouched.
pub fn filter_file_entries<'a>(entries: &'a [FileEntry], query: &str) -> Cow<'a, [FileEntry]> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Cow::Borrowed(entries);
    }
    Cow::Owned(
        entries
            .iter()
            .filter(|e| e.name.to_lowercase().contains(&q))
            .cloned()
            .collect(),
    )
}

const PREFS_KEY: &str = "devterm.fileSort.v1";

pub fn file_sort_prefs_key() -> &'static str {
    PREFS_KEY
}

/// Load persisted sort prefs. Outside a browser (no storage) this is the default.
pub fn load_file_sort_prefs() -> FileSortPrefs {
    DEFAULT_FILE_SORT
}

pub fn parse_file_sort_prefs(raw: &str) -> FileSortPrefs {
    let key = if raw.contains("\"size\"") {
        FileSortKey::Size
    } else if raw.contains("\"modified\"") {
        FileSortKey::Modified
    } else if raw.contains("\"type\"") {
        FileSortKey::Type
    } else if raw.contains("\"name\"") || raw.contains("\"key\"") {
        FileSortKey::Name
    } else {
        return DEFAULT_FILE_SORT;
    };
    // Prefer the key field specifically.
    let key = match raw.split("\"key\"").nth(1) {
        Some(rest) => {
            if rest.contains("\"size\"") {
                FileSortKey::Size
            } else if rest.contains("\"modified\"") {
                FileSortKey::Modified
            } else if rest.contains("\"type\"") {
                FileSortKey::Type
            } else if rest.contains("\"name\"") {
                FileSortKey::Name
            } else {
                key
            }
        }
        None => return DEFAULT_FILE_SORT,
    };
    let dir = if raw.contains("\"desc\"") {
        FileSortDir::Desc
    } else {
        FileSortDir::Asc
    };
    let dirs_first = !raw.contains("\"dirsFirst\":false") && !raw.contains("\"dirsFirst\": false");
    FileSortPrefs {
        key,
        dir,
        dirs_first,
    }
}

pub fn save_file_sort_prefs(prefs: FileSortPrefs) -> String {
    let key = prefs.key.as_str();
    let dir = match prefs.dir {
        FileSortDir::Asc => "asc",
        FileSortDir::Desc => "desc",
    };
    let dirs = if prefs.dirs_first { "true" } else { "false" };
    format!("{{\"key\":\"{key}\",\"dir\":\"{dir}\",\"dirsFirst\":{dirs}}}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool, size: i64, mtime_ms: i64) -> FileEntry {
        FileEntry {
            path: format!("/tmp/{name}"),
            name: name.to_string(),
            is_dir,
            is_symlink: false,
            size,
            mtime_ms,
            mode: "-rw-r--r--".into(),
        }
    }

    fn names(es: &[FileEntry]) -> Vec<String> {
        es.iter().map(|e| e.name.clone()).collect()
    }

    fn mixed() -> Vec<FileEntry> {
        vec![
            entry("b.txt", false, 20, 200),
            entry("docs", true, 0, 50),
            entry("a.txt", false, 10, 300),
            entry("img.png", false, 30, 100),
        ]
    }

    #[test]
    fn file_type_key_cases() {
        assert_eq!(file_type_key("Photo.JPG"), "jpg");
        assert_eq!(file_type_key("archive.tar.gz"), "gz");
        assert_eq!(file_type_key("Makefile"), "");
        assert_eq!(file_type_key(".gitignore"), "");
        assert_eq!(file_type_key("weird."), "");
    }

    #[test]
    fn defaults_to_dirs_first_name_ascending() {
        let mixed = mixed();
        assert_eq!(
            names(&sort_file_entries(&mixed, DEFAULT_FILE_SORT)),
            vec!["docs", "a.txt", "b.txt", "img.png"]
        );
    }

    #[test]
    fn sorts_names_naturally_and_case_insensitively() {
        let es = vec![entry("file10.txt", false, 0, 0), entry("File2.txt", false, 0, 0)];
        assert_eq!(
            names(&sort_file_entries(&es, DEFAULT_FILE_SORT)),
            vec!["File2.txt", "file10.txt"]
        );
    }

    #[test]
    fn mixes_dirs_and_files_when_dirs_first_is_off() {
        let out = sort_file_entries(
            &mixed(),
            FileSortPrefs {
                key: FileSortKey::Name,
                dir: FileSortDir::Asc,
                dirs_first: false,
            },
        );
        assert_eq!(names(&out), vec!["a.txt", "b.txt", "docs", "img.png"]);
    }

    #[test]
    fn sorts_by_size_with_a_name_tie_break() {
        let es = vec![
            entry("b", false, 10, 0),
            entry("a", false, 10, 0),
            entry("c", false, 5, 0),
        ];
        let out = sort_file_entries(
            &es,
            FileSortPrefs {
                key: FileSortKey::Size,
                dir: FileSortDir::Asc,
                dirs_first: false,
            },
        );
        assert_eq!(names(&out), vec!["c", "a", "b"]);
    }

    #[test]
    fn sorts_by_modified_time() {
        let out = sort_file_entries(
            &mixed(),
            FileSortPrefs {
                key: FileSortKey::Modified,
                dir: FileSortDir::Asc,
                dirs_first: false,
            },
        );
        assert_eq!(names(&out), vec!["docs", "img.png", "b.txt", "a.txt"]);
    }

    #[test]
    fn sorts_by_type_extensionless_first() {
        let es = vec![
            entry("b.png", false, 0, 0),
            entry("Makefile", false, 0, 0),
            entry("a.txt", false, 0, 0),
            entry("c.png", false, 0, 0),
        ];
        let out = sort_file_entries(
            &es,
            FileSortPrefs {
                key: FileSortKey::Type,
                dir: FileSortDir::Asc,
                dirs_first: false,
            },
        );
        assert_eq!(names(&out), vec!["Makefile", "b.png", "c.png", "a.txt"]);
    }

    #[test]
    fn reverses_the_whole_order_when_descending() {
        let es = vec![entry("a", false, 1, 0), entry("b", false, 1, 0)];
        let out = sort_file_entries(
            &es,
            FileSortPrefs {
                key: FileSortKey::Size,
                dir: FileSortDir::Desc,
                dirs_first: false,
            },
        );
        assert_eq!(names(&out), vec!["b", "a"]);
        let out2 = sort_file_entries(
            &mixed(),
            FileSortPrefs {
                key: FileSortKey::Name,
                dir: FileSortDir::Desc,
                dirs_first: true,
            },
        );
        assert_eq!(names(&out2), vec!["docs", "img.png", "b.txt", "a.txt"]);
    }

    #[test]
    fn keeps_dirs_grouped_first_even_when_descending() {
        let es = vec![entry("aaa", false, 0, 0), entry("zzz", true, 0, 0)];
        let out = sort_file_entries(
            &es,
            FileSortPrefs {
                key: FileSortKey::Name,
                dir: FileSortDir::Desc,
                dirs_first: true,
            },
        );
        assert_eq!(names(&out), vec!["zzz", "aaa"]);
    }

    #[test]
    fn does_not_mutate_the_input_array() {
        let mixed = mixed();
        let before = names(&mixed);
        let _ = sort_file_entries(
            &mixed,
            FileSortPrefs {
                key: FileSortKey::Size,
                dir: FileSortDir::Desc,
                dirs_first: false,
            },
        );
        assert_eq!(names(&mixed), before);
    }

    #[test]
    fn filter_file_entries_cases() {
        let es = vec![
            entry("README.md", false, 0, 0),
            entry("src", true, 0, 0),
            entry("readline.c", false, 0, 0),
        ];
        assert!(matches!(filter_file_entries(&es, ""), Cow::Borrowed(_)));
        assert!(matches!(filter_file_entries(&es, "   "), Cow::Borrowed(_)));
        assert_eq!(
            names(&filter_file_entries(&es, "read")),
            vec!["README.md", "readline.c"]
        );
        assert_eq!(names(&filter_file_entries(&es, "SRC")), vec!["src"]);
        assert!(filter_file_entries(&es, "zzz-nope").is_empty());
    }

    #[test]
    fn load_file_sort_prefs_returns_defaults_outside_a_browser() {
        assert_eq!(load_file_sort_prefs(), DEFAULT_FILE_SORT);
    }
}
