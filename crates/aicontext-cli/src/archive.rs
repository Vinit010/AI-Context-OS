//! The portable `.ai` archive: what `aicontext export` writes and `aicontext import` reads
//! (`docs/CLI_SPEC.md` §4, `TASK-020`, `ADR-008`).
//!
//! The format is one pretty-printed JSON document with three deliberate properties:
//!
//! - **Deterministic.** No timestamps, no absolute paths, no host names; files are sorted by their
//!   `/`-separated relative path and struct fields serialise in declaration order. Exporting the
//!   same tree twice produces the same bytes, which is what makes the round-trip test in
//!   `tests/import.rs` meaningful.
//! - **Verifiable.** Every file carries its byte length and its SHA-256 digest, and the manifest
//!   carries a digest over the whole list. `import` recomputes all of them before writing anything.
//!   The digests detect corruption and accidental edits; they are not a signature, so an archive
//!   stays untrusted input (`MEM-003`) and every path is validated on the way in.
//! - **Not lossy** (PRD F10): file contents are UTF-8 text stored verbatim, so a round trip on an
//!   unedited tree is byte-identical, and a reader who is not this tool still sees the documents.
//!
//! The size and depth caps are the same order of bounds `doctor`'s tree reader applies: a command
//! that reads an external file must cap what it reads (`RULES.md` §11), and the caps are checked
//! before content is accepted rather than after.

use crate::output::Finding;
use serde::{Deserialize, Serialize};

/// The `format` field every archive declares. A file without it is not one of ours.
pub(crate) const FORMAT: &str = "aicontext-archive";

/// The one archive version this binary reads and writes. A different version is refused by name
/// rather than partially interpreted.
pub(crate) const VERSION: u32 = 1;

/// The most files an archive may list, on either side of the round trip.
///
/// Mirrors `doctor`'s `MAX_DOCUMENTS`, so a tree `doctor` can read is a tree `export` can archive.
pub(crate) const MAX_ENTRIES: usize = 512;

/// The most path components an entry may have, mirroring `doctor`'s `MAX_DEPTH`.
pub(crate) const MAX_DEPTH: usize = 16;

/// The most bytes one file's content may occupy. Eight times the 1 MiB document cap, so every
/// document `doctor` tolerates is archivable while a single hostile entry stays bounded.
pub(crate) const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;

/// The most bytes the archive file itself may occupy, checked before it is read.
pub(crate) const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;

/// One archive document.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Archive {
    /// Always [`FORMAT`]; checked before anything else is read.
    pub(crate) format: String,
    /// Always [`VERSION`].
    pub(crate) version: u32,
    /// What is in the archive, and the digests that prove it.
    pub(crate) manifest: Manifest,
    /// The file contents, in the same order as [`Manifest::files`].
    pub(crate) entries: Vec<Entry>,
}

/// The manifest: the archive's own description, independent of the content it accompanies.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    /// The project name, taken from the exporting directory.
    pub(crate) project: String,
    /// How many files the manifest lists; `import` checks it against the list itself.
    pub(crate) file_count: usize,
    /// The sum of [`FileDigest::bytes`]; `import` checks it against the list itself.
    pub(crate) total_bytes: usize,
    /// One digest per file, sorted by path.
    pub(crate) files: Vec<FileDigest>,
    /// SHA-256 over the canonical spelling of [`Manifest::files`]; see [`manifest_digest`].
    pub(crate) digest: String,
}

/// One file's identity: where it lives, how long it is, and what it hashes to.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileDigest {
    /// Path relative to `.ai/`, with `/` separators.
    pub(crate) path: String,
    /// The content's length in bytes, which `import` checks against the content itself.
    pub(crate) bytes: usize,
    /// Lowercase hex SHA-256 of the content.
    pub(crate) sha256: String,
}

/// One file's content, in the manifest's order.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    /// Path relative to `.ai/`, with `/` separators. Must equal the manifest row at the same index.
    pub(crate) path: String,
    /// The file's exact UTF-8 bytes, stored as a JSON string.
    pub(crate) content: String,
}

/// Builds an archive from `(path, content)` pairs.
///
/// Sorting happens here and nowhere else: determinism is a property of the writer, so a caller that
/// hands over files in walk order still produces the archive a second run would produce.
pub(crate) fn build(project: impl Into<String>, mut files: Vec<(String, String)>) -> Archive {
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut digests = Vec::with_capacity(files.len());
    let mut entries = Vec::with_capacity(files.len());
    let mut total_bytes = 0;
    for (path, content) in files {
        total_bytes += content.len();
        digests.push(FileDigest {
            path: path.clone(),
            bytes: content.len(),
            sha256: sha256_hex(content.as_bytes()),
        });
        entries.push(Entry { path, content });
    }

    let digest = manifest_digest(&digests);
    Archive {
        format: FORMAT.to_string(),
        version: VERSION,
        manifest: Manifest {
            project: project.into(),
            file_count: digests.len(),
            total_bytes,
            files: digests,
            digest,
        },
        entries,
    }
}

/// Renders an archive to the exact bytes `export` writes: pretty JSON, one trailing newline.
///
/// Pretty printing is not cosmetic here. The whole point of a single document is that a human can
/// read it and `diff` it, and a stable layout is also what makes two exports byte-comparable.
pub(crate) fn render(archive: &Archive) -> Result<String, serde_json::Error> {
    let mut text = serde_json::to_string_pretty(archive)?;
    text.push('\n');
    Ok(text)
}

/// Anything that stops a file from being read as an archive this version accepts.
///
/// These are all *unreadability*: the file cannot be used at all, which `import` maps to a usage
/// failure (exit 2). A file that parses but does not verify is a different outcome and becomes
/// findings instead.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ArchiveProblem {
    /// The bytes are not a JSON document, or not a document of this shape.
    #[error("it is not a JSON archive document: {source}")]
    NotJson {
        /// What the parser said.
        #[source]
        source: serde_json::Error,
    },

    /// The document declares another format.
    #[error("it declares the format {found:?}, not {FORMAT:?}")]
    WrongFormat {
        /// The `format` field the file carries.
        found: String,
    },

    /// The document declares another version.
    #[error("it declares version {found}, and this binary reads version {VERSION}")]
    WrongVersion {
        /// The `version` the file carries.
        found: u32,
    },

    /// The archive lists more entries than this command will read.
    #[error("it lists {count} entries, over the {limit}-entry limit")]
    TooManyEntries {
        /// How many it lists.
        count: usize,
        /// The limit it broke.
        limit: usize,
    },

    /// One entry's content is larger than this command will read.
    #[error("the entry {path} holds {bytes} bytes, over the {limit}-byte per-file limit")]
    FileTooLarge {
        /// The offending entry.
        path: String,
        /// Its content length.
        bytes: usize,
        /// The limit it broke.
        limit: usize,
    },
}

/// Reads archive bytes that were already known to be small enough to read.
pub(crate) fn parse(text: &str) -> Result<Archive, ArchiveProblem> {
    let archive: Archive =
        serde_json::from_str(text).map_err(|source| ArchiveProblem::NotJson { source })?;

    if archive.format != FORMAT {
        return Err(ArchiveProblem::WrongFormat {
            found: archive.format,
        });
    }
    if archive.version != VERSION {
        return Err(ArchiveProblem::WrongVersion {
            found: archive.version,
        });
    }
    if archive.manifest.files.len() > MAX_ENTRIES || archive.entries.len() > MAX_ENTRIES {
        return Err(ArchiveProblem::TooManyEntries {
            count: archive.manifest.files.len().max(archive.entries.len()),
            limit: MAX_ENTRIES,
        });
    }
    for entry in &archive.entries {
        if entry.content.len() > MAX_FILE_BYTES {
            return Err(ArchiveProblem::FileTooLarge {
                path: entry.path.clone(),
                bytes: entry.content.len(),
                limit: MAX_FILE_BYTES,
            });
        }
    }
    Ok(archive)
}

/// Verifies a parsed archive against its own manifest, and returns one finding per problem.
///
/// `label` is the archive's path as a person would recognise it, used as the finding path for
/// problems with the document as a whole. Nothing here reads or writes the filesystem: verification
/// is pure, so `import` can run it before it plans a single write (`RULES.md` §7, default deny).
///
/// The checks are ordered so a broken structure does not produce a finding per pair: the pairwise
/// and per-file checks only run when the manifest and the entries are known to line up.
pub(crate) fn verify(archive: &Archive, label: &str) -> Vec<Finding> {
    let files = &archive.manifest.files;
    let entries = &archive.entries;

    let mut findings = Vec::new();
    let aligned = manifest_shape(archive, label, &mut findings);
    unsafe_paths(files, label, &mut findings);
    duplicate_paths(files, label, &mut findings);
    if aligned {
        verify_pairs(files, entries, label, &mut findings);
    }
    if manifest_digest(files) != archive.manifest.digest {
        findings.push(Finding::new(
            "IMP-010",
            "error",
            label,
            "the manifest digest does not match the manifest's own file list",
            "the archive was altered or truncated; re-export or re-download it",
        ));
    }
    findings
}

/// Reconciles the manifest's counts with the lists it travels with. When any count is off, the
/// pairwise checks below cannot trust the pairing, so `aligned` is false and they are skipped.
fn manifest_shape(archive: &Archive, label: &str, findings: &mut Vec<Finding>) -> bool {
    let files = &archive.manifest.files;
    let entries = &archive.entries;

    let mut aligned = true;
    if archive.manifest.file_count != files.len() {
        findings.push(Finding::new(
            "IMP-014",
            "error",
            label,
            format!(
                "the manifest says {} files but lists {}",
                archive.manifest.file_count,
                files.len()
            ),
            "the archive is incomplete; re-export it from the source project",
        ));
        aligned = false;
    }
    if files.len() != entries.len() {
        findings.push(Finding::new(
            "IMP-014",
            "error",
            label,
            format!(
                "the manifest lists {} files but the archive carries {} entries",
                files.len(),
                entries.len()
            ),
            "the archive is incomplete; re-export it from the source project",
        ));
        aligned = false;
    }
    let declared_total: usize = files.iter().map(|file| file.bytes).sum();
    if archive.manifest.total_bytes != declared_total {
        findings.push(Finding::new(
            "IMP-014",
            "error",
            label,
            format!(
                "the manifest claims {} bytes in total but its own rows add up to {}",
                archive.manifest.total_bytes, declared_total
            ),
            "the archive is inconsistent; re-export it from the source project",
        ));
    }
    aligned
}

/// Every archive path must be one this command would write under `.ai/`.
fn unsafe_paths(files: &[FileDigest], label: &str, findings: &mut Vec<Finding>) {
    for file in files {
        if !safe_relative(&file.path) {
            findings.push(Finding::new(
                "IMP-013",
                "error",
                label,
                format!(
                    "the entry {:?} is not a path this command will write under .ai/",
                    file.path
                ),
                "the archive is not safe to import; re-export it from a trusted project",
            ));
        }
    }
}

/// A path listed twice would import the same content twice, depending on sort order.
fn duplicate_paths(files: &[FileDigest], label: &str, findings: &mut Vec<Finding>) {
    let mut sorted: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
    sorted.sort_unstable();
    if let Some(previous) = sorted
        .windows(2)
        .find(|pair| pair[0] == pair[1])
        .map(|pair| pair[0])
    {
        findings.push(Finding::new(
            "IMP-015",
            "error",
            label,
            format!("the entry {previous:?} appears more than once"),
            "the archive is ambiguous; re-export it from the source project",
        ));
    }
}

/// Checks the matching of each manifest row against the entry it travels with, and the content
/// against the row's digest. Only called when the counts line up.
fn verify_pairs(files: &[FileDigest], entries: &[Entry], label: &str, findings: &mut Vec<Finding>) {
    for (digest, entry) in files.iter().zip(entries) {
        if digest.path != entry.path {
            findings.push(Finding::new(
                "IMP-014",
                "error",
                label,
                format!(
                    "the manifest row {:?} and the entry {:?} are in different positions",
                    digest.path, entry.path
                ),
                "the archive is inconsistent; re-export it from the source project",
            ));
            continue;
        }
        if digest.bytes != entry.content.len() {
            findings.push(Finding::new(
                "IMP-012",
                "error",
                format!(".ai/{}", entry.path),
                format!(
                    "the manifest says {} bytes but the content holds {}",
                    digest.bytes,
                    entry.content.len()
                ),
                "the archive is corrupt; re-export it from the source project",
            ));
        }
        let actual = sha256_hex(entry.content.as_bytes());
        if actual != digest.sha256 {
            findings.push(Finding::new(
                "IMP-011",
                "error",
                format!(".ai/{}", entry.path),
                "the content does not match the digest in the manifest",
                "the archive was altered or truncated; re-export or re-download it",
            ));
        }
    }
}

/// Whether a path from an archive is one this command will write under `.ai/`.
///
/// Refused: the empty path, anything over a reasonable length, an absolute path, a backslash or a
/// colon (which is a drive letter or an alternate data stream on Windows), any control character
/// including the line breaks a report would be confused by, and any component that is empty, `.`,
/// or `..`. Depth is capped at [`MAX_DEPTH`] so a hostile archive cannot build a deeper tree than
/// `doctor` reads.
///
/// The check is shared: `export` runs it over its own walk so it can never write an archive that
/// `import` would refuse (`TASK-020`).
pub(crate) fn safe_relative(path: &str) -> bool {
    if path.is_empty() || path.len() > 1024 {
        return false;
    }
    if path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return false;
    }
    if path.chars().any(char::is_control) {
        return false;
    }
    let mut components = 0;
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return false;
        }
        components += 1;
    }
    components <= MAX_DEPTH
}

/// The manifest's digest: SHA-256 over one canonical line per file, in manifest order.
///
/// Hand-spelled rather than `serde_json::to_string` on the list, so the function is total — no
/// `unwrap` in library code (`RULES.md` §3) — and so the digest's input is a format we define here
/// instead of whatever a serialiser decides to emit. The spelling is `path`, `bytes`, and `sha256`
/// separated by newlines; every field is guaranteed newline-free because [`safe_relative`] rejects
/// control characters and the other two are a number and a hex string.
pub(crate) fn manifest_digest(files: &[FileDigest]) -> String {
    let mut canonical = String::new();
    for file in files {
        canonical.push_str(&file.path);
        canonical.push('\n');
        canonical.push_str(&file.bytes.to_string());
        canonical.push('\n');
        canonical.push_str(&file.sha256);
        canonical.push('\n');
    }
    sha256_hex(canonical.as_bytes())
}

/// SHA-256 of `data`, as lowercase hex.
///
/// This is the hash `ARCHITECTURE.md` §2.2 pre-justifies for content hashing: a digest in a
/// portable archive has to be one another tool can reproduce, so a non-cryptographic hash would not
/// be an honest answer. The hex encoding is this function's six lines rather than a `hex`
/// dependency (`RULES.md` §6).
pub(crate) fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;

    let digest = Sha256::digest(data);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{
        ArchiveProblem, FORMAT, FileDigest, MAX_ARCHIVE_BYTES, MAX_ENTRIES, MAX_FILE_BYTES,
        VERSION, build, manifest_digest, parse, render, safe_relative, sha256_hex, verify,
    };

    fn sample() -> super::Archive {
        build(
            "ledger".to_string(),
            vec![
                ("RULES.md".to_string(), "# rules\n".to_string()),
                ("AI.md".to_string(), "# ai\n".to_string()),
            ],
        )
    }

    #[test]
    fn sha256_matches_the_published_vector_for_abc() {
        // FIPS 180-2 test vector: a digest algorithm that "almost" matches is worse than none.
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn building_is_deterministic_and_order_independent() {
        let forward = build(
            "ledger".to_string(),
            vec![
                ("AI.md".to_string(), "# ai\n".to_string()),
                ("RULES.md".to_string(), "# rules\n".to_string()),
            ],
        );
        let backward = build(
            "ledger".to_string(),
            vec![
                ("RULES.md".to_string(), "# rules\n".to_string()),
                ("AI.md".to_string(), "# ai\n".to_string()),
            ],
        );
        assert_eq!(
            render(&forward).expect("renders"),
            render(&backward).expect("renders")
        );
        assert_eq!(forward, backward);
    }

    #[test]
    fn a_rendered_archive_parses_back_and_renders_identically() {
        let archive = sample();
        let text = render(&archive).expect("renders");
        let reparsed = parse(&text).expect("parses");
        assert_eq!(reparsed, archive);
        assert_eq!(
            render(&reparsed).expect("renders again"),
            text,
            "a round trip must be byte-identical"
        );
    }

    #[test]
    fn the_manifest_digest_follows_the_file_list() {
        let one = vec![FileDigest {
            path: "AI.md".to_string(),
            bytes: 4,
            sha256: "aa".to_string(),
        }];
        let mut other = one.clone();
        other[0].sha256 = "bb".to_string();
        assert_ne!(
            manifest_digest(&one),
            manifest_digest(&other),
            "a changed digest must change the manifest digest"
        );
        assert_eq!(manifest_digest(&one), manifest_digest(&one.clone()));
        assert_eq!(manifest_digest(&[]), sha256_hex(b""));
    }

    #[test]
    fn parse_refuses_what_it_cannot_read() {
        let problem = parse("not json").expect_err("not JSON");
        assert!(matches!(problem, ArchiveProblem::NotJson { .. }));

        let mut archive = sample();
        archive.format = "tarball".to_string();
        let problem = parse(&render(&archive).expect("renders")).expect_err("wrong format");
        assert!(matches!(
            problem,
            ArchiveProblem::WrongFormat { ref found } if found == "tarball"
        ));

        let mut archive = sample();
        archive.version = VERSION + 1;
        let problem = parse(&render(&archive).expect("renders")).expect_err("wrong version");
        assert!(matches!(
            problem,
            ArchiveProblem::WrongVersion { found } if found == VERSION + 1
        ));

        let mut archive = sample();
        archive.manifest.files.clear();
        archive.entries.clear();
        for index in 0..=MAX_ENTRIES {
            let path = format!("f{index}.md");
            archive.manifest.files.push(FileDigest {
                path: path.clone(),
                bytes: 0,
                sha256: String::new(),
            });
            archive.entries.push(super::Entry {
                path,
                content: String::new(),
            });
        }
        let problem = parse(&render(&archive).expect("renders")).expect_err("too many entries");
        assert!(matches!(
            problem,
            ArchiveProblem::TooManyEntries { count, limit } if count == MAX_ENTRIES + 1 && limit == MAX_ENTRIES
        ));

        let archive = build(
            "ledger".to_string(),
            vec![("AI.md".to_string(), "x".repeat(MAX_FILE_BYTES + 1))],
        );
        let problem = parse(&render(&archive).expect("renders")).expect_err("oversize entry");
        assert!(matches!(
            problem,
            ArchiveProblem::FileTooLarge { bytes, limit, .. }
                if bytes == MAX_FILE_BYTES + 1 && limit == MAX_FILE_BYTES
        ));
    }

    #[test]
    fn every_problem_message_says_what_is_wrong_and_stays_short() {
        for problem in [
            ArchiveProblem::NotJson {
                source: serde_json::from_str::<()>("{").expect_err("bad json"),
            },
            ArchiveProblem::WrongFormat {
                found: "zip".to_string(),
            },
            ArchiveProblem::WrongVersion { found: 9 },
            ArchiveProblem::TooManyEntries { count: 9, limit: 8 },
            ArchiveProblem::FileTooLarge {
                path: "AI.md".to_string(),
                bytes: 9,
                limit: 8,
            },
        ] {
            let text = problem.to_string();
            assert!(!text.is_empty(), "{problem:?} must explain itself");
            assert!(
                text.len() < 300,
                "an error is a sentence, not a paragraph: {text}"
            );
        }
    }

    #[test]
    fn safe_relative_draws_the_line_where_import_would() {
        for accepted in [
            "AI.md",
            "schemas/task.schema.json",
            "context/stack.md",
            "a file with spaces.md",
            "unicode-☃.md",
        ] {
            assert!(safe_relative(accepted), "{accepted} must be accepted");
        }
        for refused in [
            "",
            "/etc/passwd",
            "../secrets.md",
            "a/../b.md",
            "./AI.md",
            "a//b.md",
            "a\\b.md",
            "C:/x.md",
            "sub:stream.md",
            "line\nbreak.md",
            "tab\tname.md",
            &format!("{}file.md", "deep/".repeat(17)),
        ] {
            assert!(!safe_relative(refused), "{refused:?} must be refused");
        }
    }

    #[test]
    fn verification_names_the_specific_problem() {
        let archive = sample();

        let mut tampered = archive.clone();
        tampered.entries[0].content = "# oi\n".to_string();
        let findings = verify(&tampered, "backup.aix");
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].code, "IMP-011");
        assert_eq!(findings[0].severity, "error");

        let mut wrong_size = archive.clone();
        wrong_size.manifest.files[0].bytes += 1;
        wrong_size.manifest.total_bytes += 1;
        let verified = verify(&wrong_size, "backup.aix");
        let codes: Vec<&str> = verified
            .iter()
            .map(|finding| finding.code.as_str())
            .collect();
        assert!(codes.contains(&"IMP-012"), "{codes:?}");

        let mut forged = archive.clone();
        forged.manifest.files[0].sha256 = "deadbeef".repeat(8);
        let verified = verify(&forged, "backup.aix");
        let codes: Vec<&str> = verified
            .iter()
            .map(|finding| finding.code.as_str())
            .collect();
        assert!(
            codes.contains(&"IMP-010"),
            "the manifest digest covers the list: {codes:?}"
        );

        let mut hostile = archive.clone();
        hostile.manifest.files[0].path = "../../etc/passwd".to_string();
        hostile.entries[0].path = "../../etc/passwd".to_string();
        let verified = verify(&hostile, "backup.aix");
        let codes: Vec<&str> = verified
            .iter()
            .map(|finding| finding.code.as_str())
            .collect();
        assert!(codes.contains(&"IMP-013"), "{codes:?}");

        let mut duplicated = archive.clone();
        duplicated
            .manifest
            .files
            .push(duplicated.manifest.files[0].clone());
        duplicated.entries.push(duplicated.entries[0].clone());
        duplicated.manifest.file_count = duplicated.manifest.files.len();
        let verified = verify(&duplicated, "backup.aix");
        let codes: Vec<&str> = verified
            .iter()
            .map(|finding| finding.code.as_str())
            .collect();
        assert!(codes.contains(&"IMP-015"), "{codes:?}");

        let mut truncated = archive.clone();
        truncated.entries.pop();
        let verified = verify(&truncated, "backup.aix");
        let codes: Vec<&str> = verified
            .iter()
            .map(|finding| finding.code.as_str())
            .collect();
        assert!(codes.contains(&"IMP-014"), "{codes:?}");

        assert!(
            verify(&archive, "backup.aix").is_empty(),
            "an untouched archive verifies clean"
        );
    }

    #[test]
    fn an_unreadable_archive_problem_reports_a_limit_a_person_can_act_on() {
        assert_eq!(MAX_ARCHIVE_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_ENTRIES, 512);
        assert_eq!(FORMAT, "aicontext-archive");
    }
}
