//! The append-only JSONL file: reading, verifying, and appending.
//!
//! The file is `.aicontext/audit.jsonl` in a project. One JSON object per line, `seq` starting at 1,
//! `prev` starting at [`GENESIS`](crate::GENESIS). Every read verifies the whole chain; there is no
//! "trusted fast path" that skips the check, because a reader that can skip verification is a reader
//! an attacker only has to fool once.
//!
//! # Concurrency
//!
//! Appending is a read-then-write under the assumption of one writer per project. Two `aicontext`
//! processes appending at the same instant can choose the same `seq`; the second record then fails
//! verification on the next read. The product has a single `LOCKED` exit code for the project lock
//! that will own this rule when commands start writing records; until then, `append` documents the
//! assumption rather than pretending a lock exists.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::error::{AuditError, AuditProblem};
use crate::hash::{GENESIS, record_digest};
use crate::record::{AuditEntry, AuditRecord, validate};
use crate::timestamp::Timestamp;

/// The directory under the project root that holds local, git-ignored state.
pub const DEFAULT_DIR: &str = ".aicontext";
/// The log's file name inside [`DEFAULT_DIR`].
pub const DEFAULT_FILE: &str = "audit.jsonl";

/// The largest log a command will read, in bytes.
///
/// A bound rather than a promise about how much history fits: 64 MiB is millions of one-line tool
/// calls, and a file beyond it is more likely to be damage or an attack than a log a human wants
/// rendered. Archiving and starting a new chain is the documented answer.
pub const MAX_LOG_BYTES: u64 = 64 * 1024 * 1024;

/// A verified chain of records.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chain {
    records: Vec<AuditRecord>,
    head: String,
}

impl Chain {
    /// Every record, in file order.
    #[must_use]
    pub fn records(&self) -> &[AuditRecord] {
        &self.records
    }

    /// The digest of the last record, or [`GENESIS`] when the chain is empty.
    #[must_use]
    pub fn head(&self) -> &str {
        &self.head
    }

    /// The number of records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the chain holds no records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// The result of verifying a log, including the verified prefix when the chain is broken.
///
/// A broken chain is not an error type here because the caller usually has two jobs at once: refuse
/// to extend the log, and tell a human how much of it did verify. The prefix in [`records`](Self::records)
/// is exactly the records that passed every check, in order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Verification {
    records: Vec<AuditRecord>,
    head: String,
    problem: Option<AuditProblem>,
}

impl Verification {
    /// Whether every record verified and the chain links from [`GENESIS`] to `head`.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.problem.is_none()
    }

    /// The verified prefix: every record when the chain is valid, the records before the break
    /// otherwise.
    #[must_use]
    pub fn records(&self) -> &[AuditRecord] {
        &self.records
    }

    /// The digest of the last verified record, or [`GENESIS`] when none verified.
    #[must_use]
    pub fn head(&self) -> &str {
        &self.head
    }

    /// What broke the chain, when it is broken.
    #[must_use]
    pub fn problem(&self) -> Option<&AuditProblem> {
        self.problem.as_ref()
    }

    /// How many records verified.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether no record verified.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// A log file at one path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    /// A log at an explicit path.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The log a project keeps at `<root>/.aicontext/audit.jsonl`.
    #[must_use]
    pub fn in_project(root: impl AsRef<Path>) -> Self {
        Self::new(root.as_ref().join(DEFAULT_DIR).join(DEFAULT_FILE))
    }

    /// The path this log reads and writes.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads and verifies the whole log.
    ///
    /// A missing file is an empty, valid chain: no audit record has been written yet is not the same
    /// as a damaged log. Every other failure to read is returned, never reported as emptiness.
    pub fn read(&self) -> Result<Chain, AuditError> {
        let scan = self.scan();
        match scan.problem {
            Some(problem) => Err(problem),
            None => Ok(Chain {
                records: scan.records,
                head: scan.head,
            }),
        }
    }

    /// Verifies the log and keeps the verified prefix even when the chain is broken.
    #[must_use]
    pub fn verify(&self) -> Verification {
        let scan = self.scan();
        Verification {
            records: scan.records,
            head: scan.head,
            problem: scan.problem.map(|problem| problem.problem()),
        }
    }

    /// Appends one event, stamped with the current instant.
    ///
    /// The existing log is verified first. A broken chain, an unreadable file, or a malformed line
    /// refuses the append with [`AuditError::AppendRefused`] (`AUD-005`) and leaves the file
    /// untouched.
    pub fn append(&self, entry: &AuditEntry) -> Result<AuditRecord, AuditError> {
        self.append_at(entry, Timestamp::now())
    }

    /// Appends one event at an explicit instant.
    ///
    /// Production callers use [`append`](Self::append). This entry point exists so tests and replay
    /// tooling can produce the same bytes twice; the timestamp is still validated like every other
    /// field.
    pub fn append_at(
        &self,
        entry: &AuditEntry,
        timestamp: Timestamp,
    ) -> Result<AuditRecord, AuditError> {
        let scan = self.scan();
        if let Some(problem) = scan.problem {
            return Err(AuditError::append_refused(problem));
        }

        let seq = match u64::try_from(scan.records.len()) {
            Ok(count) => count.saturating_add(1),
            Err(_) => u64::MAX,
        };
        let prev = scan
            .records
            .last()
            .map_or(GENESIS, AuditRecord::digest)
            .to_string();
        let record = AuditRecord::seal(seq, timestamp, entry, &prev)?;
        let line = serde_json::to_string(&record).map_err(|error| {
            AuditError::invalid_record(format!("the record cannot be rendered as JSON: {error}"))
        })?;

        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|source| AuditError::Write {
                    path: self.display_path(),
                    source,
                })?;
            }
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| AuditError::Write {
                path: self.display_path(),
                source,
            })?;
        file.write_all(line.as_bytes())
            .and_then(|()| file.write_all(b"\n"))
            // A record acknowledged to the caller must survive a crash. The cost is one sync per tool
            // call, which is irrelevant next to the call itself.
            .and_then(|()| file.sync_data())
            .map_err(|source| AuditError::Write {
                path: self.display_path(),
                source,
            })?;
        Ok(record)
    }

    /// Reads and verifies the file, keeping the verified prefix on failure.
    fn scan(&self) -> Scan {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Scan::empty();
            }
            Err(source) => {
                return Scan::failed(AuditError::Read {
                    path: self.display_path(),
                    source,
                });
            }
        };

        let metadata = match file.metadata() {
            Ok(metadata) => metadata,
            Err(source) => {
                return Scan::failed(AuditError::Read {
                    path: self.display_path(),
                    source,
                });
            }
        };
        if metadata.len() > MAX_LOG_BYTES {
            return Scan::failed(AuditError::TooLarge {
                path: self.display_path(),
                size: metadata.len(),
                limit: MAX_LOG_BYTES,
            });
        }

        let mut text = String::new();
        let mut limited = Read::by_ref(&mut file).take(MAX_LOG_BYTES + 1);
        if let Err(error) = limited.read_to_string(&mut text) {
            return Scan::failed(if error.kind() == std::io::ErrorKind::InvalidData {
                AuditError::Invalid {
                    line: None,
                    message: format!(
                        "the audit log `{}` is not valid UTF-8: {error}",
                        self.display_path()
                    ),
                }
            } else {
                AuditError::Read {
                    path: self.display_path(),
                    source: error,
                }
            });
        }
        if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAX_LOG_BYTES {
            // The file can grow between the metadata check and the read. The cap is a bound on what
            // this process parses, not a promise about the directory entry.
            return Scan::failed(AuditError::TooLarge {
                path: self.display_path(),
                size: u64::try_from(text.len()).unwrap_or(u64::MAX),
                limit: MAX_LOG_BYTES,
            });
        }

        let mut records = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let line_number = index + 1;
            if line.trim().is_empty() {
                continue;
            }
            let record = match serde_json::from_str::<AuditRecord>(line) {
                Ok(record) => record,
                Err(source) => {
                    return Scan::partial(
                        records,
                        AuditError::Malformed {
                            line: line_number,
                            source,
                        },
                    );
                }
            };
            if let Err(message) = validate(&record) {
                return Scan::partial(
                    records,
                    AuditError::Invalid {
                        line: Some(line_number),
                        message: format!(
                            "line {line_number} is not a valid audit record: {message}"
                        ),
                    },
                );
            }

            let expected_seq = match u64::try_from(records.len()) {
                Ok(count) => count.saturating_add(1),
                Err(_) => u64::MAX,
            };
            if record.seq() != expected_seq {
                return Scan::partial(
                    records,
                    AuditError::Broken {
                        line: line_number,
                        message: format!(
                            "seq is {} but the next record must be {expected_seq}",
                            record.seq()
                        ),
                    },
                );
            }
            let expected_prev = records
                .last()
                .map_or(GENESIS, AuditRecord::digest)
                .to_string();
            if record.prev() != expected_prev {
                return Scan::partial(
                    records,
                    AuditError::Broken {
                        line: line_number,
                        message: format!(
                            "prev is {} but the previous record's self is {expected_prev}",
                            record.prev()
                        ),
                    },
                );
            }
            let expected_digest = match record_digest(&record) {
                Ok(digest) => digest,
                Err(error) => return Scan::partial(records, error),
            };
            if record.digest() != expected_digest {
                return Scan::partial(
                    records,
                    AuditError::Broken {
                        line: line_number,
                        message: format!(
                            "self is {} but the record's contents digest to {expected_digest}",
                            record.digest()
                        ),
                    },
                );
            }
            records.push(record);
        }

        let head = records
            .last()
            .map_or(GENESIS, AuditRecord::digest)
            .to_string();
        Scan {
            records,
            head,
            problem: None,
        }
    }

    /// The path as a human reads it, with native separators removed from the message only.
    fn display_path(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

/// One scan's outcome: the verified prefix, its head, and the failure after it.
struct Scan {
    records: Vec<AuditRecord>,
    head: String,
    problem: Option<AuditError>,
}

impl Scan {
    fn empty() -> Self {
        Self {
            records: Vec::new(),
            head: GENESIS.to_string(),
            problem: None,
        }
    }

    fn failed(problem: AuditError) -> Self {
        Self {
            records: Vec::new(),
            head: GENESIS.to_string(),
            problem: Some(problem),
        }
    }

    fn partial(records: Vec<AuditRecord>, problem: AuditError) -> Self {
        let head = records
            .last()
            .map_or(GENESIS, AuditRecord::digest)
            .to_string();
        Self {
            records,
            head,
            problem: Some(problem),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use serde_json::Value;
    use tempfile::TempDir;

    use super::{AuditLog, MAX_LOG_BYTES};
    use crate::record::{Action, Actor, Approval, AuditEntry, ResultSummary};
    use crate::{GENESIS, Timestamp};

    fn log() -> (TempDir, AuditLog) {
        let directory = TempDir::new().expect("temp dir");
        let log = AuditLog::new(directory.path().join(".aicontext/audit.jsonl"));
        (directory, log)
    }

    fn entry(tool: &str) -> AuditEntry {
        AuditEntry::new(
            "completed",
            Actor::agent("developer"),
            Action::new(tool, "aws", "write"),
        )
        .with_resource("i-0abc1234")
        .with_arguments_digest(super::super::hash::sha256(b"{\"instance\":\"i-0abc1234\"}"))
        .with_approval(
            Approval::new(aicontext_core::PermissionMode::Approval)
                .with_granted_by("developer")
                .with_mode("interactive"),
        )
        .with_result(ResultSummary::new("success").with_duration_ms(412))
    }

    fn at(millis: i64) -> Timestamp {
        Timestamp::from_unix_millis(millis)
    }

    fn read_file(log: &AuditLog) -> String {
        fs::read_to_string(log.path()).expect("the log exists")
    }

    fn write_file(log: &AuditLog, text: &str) {
        if let Some(parent) = log.path().parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(log.path(), text).expect("the log is written");
    }

    fn lines(log: &AuditLog) -> Vec<String> {
        read_file(log).lines().map(str::to_string).collect()
    }

    #[test]
    fn a_missing_log_is_an_empty_valid_chain() {
        let (_directory, log) = log();
        let chain = log.read().expect("missing is empty");
        assert!(chain.is_empty());
        assert_eq!(chain.head(), GENESIS);
        let verification = log.verify();
        assert!(verification.is_valid());
        assert!(verification.is_empty());
        assert_eq!(verification.head(), GENESIS);
        assert!(verification.problem().is_none());
    }

    #[test]
    fn an_empty_file_is_an_empty_valid_chain() {
        let (_directory, log) = log();
        write_file(&log, "");
        assert!(log.read().expect("empty file").is_empty());
    }

    #[test]
    fn append_writes_the_first_record_at_seq_one_after_genesis() {
        let (_directory, log) = log();
        let record = log
            .append_at(&entry("aws.ec2.stop"), at(0))
            .expect("appends");
        assert_eq!(record.seq(), 1);
        assert_eq!(record.prev(), GENESIS);
        assert!(record.digest().starts_with("sha256:"));

        let chain = log.read().expect("reads");
        assert_eq!(chain.len(), 1);
        assert_eq!(chain.head(), record.digest());
        assert_eq!(chain.records()[0], record);
    }

    #[test]
    fn append_links_each_record_to_the_one_before_it() {
        let (_directory, log) = log();
        let first = log.append_at(&entry("aws.ec2.stop"), at(0)).expect("first");
        let second = log
            .append_at(&entry("aws.ec2.start"), at(1))
            .expect("second");
        let third = log
            .append_at(&entry("aws.ec2.describe"), at(2))
            .expect("third");

        assert_eq!(second.seq(), 2);
        assert_eq!(second.prev(), first.digest());
        assert_eq!(third.seq(), 3);
        assert_eq!(third.prev(), second.digest());
        assert_eq!(log.verify().head(), third.digest());
    }

    #[test]
    fn the_file_is_one_json_object_per_line_ending_with_a_newline() {
        let (_directory, log) = log();
        log.append_at(&entry("aws.ec2.stop"), at(0))
            .expect("append");
        let text = read_file(&log);
        assert!(text.ends_with('\n'));
        assert_eq!(text.lines().count(), 1);
        let value: Value = serde_json::from_str(text.trim()).expect("one object");
        assert_eq!(value["seq"], 1);
        assert_eq!(value["prev"], GENESIS);
        assert!(value.get("self").is_some());
    }

    #[test]
    fn the_same_entry_at_the_same_instant_produces_the_same_bytes_twice() {
        // `self` is a digest of canonical JSON, so two independent logs agree byte for byte. This is
        // the property a reviewer needs to reproduce a record from a written description.
        let (first_dir, first) = log();
        let (_second_dir, second) = log();
        let first_record = first
            .append_at(&entry("aws.ec2.stop"), at(1_790_000_000_412))
            .expect("first");
        let second_record = second
            .append_at(&entry("aws.ec2.stop"), at(1_790_000_000_412))
            .expect("second");
        assert_eq!(first_record, second_record);
        assert_eq!(read_file(&first), read_file(&second));
        drop(first_dir);
    }

    #[test]
    fn a_log_with_a_reordered_key_is_still_valid() {
        // The digest is over sorted canonical JSON, not over the bytes of the line, so a reader that
        // re-renders the same values in a different order must not be told the chain is broken.
        let (_directory, log) = log();
        log.append_at(&entry("aws.ec2.stop"), at(0))
            .expect("append");
        let line = lines(&log)[0].clone();
        let value: Value = serde_json::from_str(&line).expect("object");
        let reordered = serde_json::to_string(&value).expect("renders");
        assert_ne!(line, reordered, "the rendered order is sorted and differs");
        write_file(&log, &format!("{reordered}\n"));
        assert!(
            log.verify().is_valid(),
            "key order is not part of the digest"
        );
    }

    #[test]
    fn changing_a_value_breaks_the_self_digest() {
        let (_directory, log) = log();
        log.append_at(&entry("aws.ec2.stop"), at(0))
            .expect("append");
        let mut value: Value = serde_json::from_str(lines(&log)[0].as_str()).expect("object");
        value["action"]["tool"] = Value::String("aws.ec2.terminate".to_string());
        write_file(&log, &format!("{value}\n"));

        let error = log.read().expect_err("tampering is detected");
        assert_eq!(error.code(), aicontext_core::ErrorCode::AUD_003);
        assert!(error.to_string().contains("self"));
        let verification = log.verify();
        assert!(!verification.is_valid());
        assert!(verification.records().is_empty());
        assert_eq!(verification.head(), GENESIS);
        assert_eq!(verification.problem().expect("problem").line(), Some(1));
    }

    #[test]
    fn changing_a_value_and_recomputing_self_breaks_the_next_prev_link() {
        let (_directory, log) = log();
        let first = log.append_at(&entry("aws.ec2.stop"), at(0)).expect("first");
        log.append_at(&entry("aws.ec2.start"), at(1))
            .expect("second");

        let mut first_value: Value = serde_json::from_str(lines(&log)[0].as_str()).expect("object");
        first_value["action"]["tool"] = Value::String("aws.ec2.terminate".to_string());
        let tampered: crate::AuditRecord =
            serde_json::from_value(first_value).expect("still a record shape");
        let recomputed = super::super::hash::record_digest(&tampered).expect("digest");
        assert_ne!(
            recomputed,
            first.digest(),
            "the tamper changed the record's self"
        );
        let mut sealed = serde_json::to_value(tampered).expect("renders");
        sealed["self"] = Value::String(recomputed.clone());
        let second_line = lines(&log)[1].clone();
        write_file(
            &log,
            &format!(
                "{}\n{second_line}\n",
                serde_json::to_string(&sealed).expect("renders")
            ),
        );

        let verification = log.verify();
        assert!(!verification.is_valid());
        assert_eq!(
            verification.records().len(),
            1,
            "the first record still verifies"
        );
        assert_eq!(verification.records()[0].digest(), recomputed);
        let problem = verification.problem().expect("problem");
        assert_eq!(problem.line(), Some(2));
        assert!(problem.message().contains("prev"));
    }

    #[test]
    fn deleting_the_middle_record_breaks_the_sequence() {
        let (_directory, log) = log();
        log.append_at(&entry("one"), at(0)).expect("one");
        log.append_at(&entry("two"), at(1)).expect("two");
        log.append_at(&entry("three"), at(2)).expect("three");
        let all = lines(&log);
        write_file(&log, &format!("{}\n{}\n", all[0], all[2]));

        let verification = log.verify();
        assert!(!verification.is_valid());
        assert_eq!(verification.records().len(), 1);
        let problem = verification.problem().expect("problem");
        assert_eq!(problem.line(), Some(2));
        assert!(problem.message().contains("seq"));
    }

    #[test]
    fn a_duplicate_or_out_of_order_sequence_is_refused() {
        let (_directory, log) = log();
        log.append_at(&entry("one"), at(0)).expect("one");
        log.append_at(&entry("two"), at(1)).expect("two");
        let all = lines(&log);
        let mut second: Value = serde_json::from_str(&all[1]).expect("object");
        second["seq"] = Value::from(1);
        write_file(
            &log,
            &format!(
                "{}\n{}\n",
                all[0],
                serde_json::to_string(&second).expect("renders")
            ),
        );
        let verification = log.verify();
        assert!(!verification.is_valid());
        assert_eq!(verification.records().len(), 1);
    }

    #[test]
    fn a_log_that_does_not_start_at_one_is_broken() {
        let (_directory, log) = log();
        log.append_at(&entry("one"), at(0)).expect("one");
        let mut value: Value = serde_json::from_str(lines(&log)[0].as_str()).expect("object");
        value["seq"] = Value::from(2);
        write_file(&log, &format!("{value}\n"));
        let verification = log.verify();
        assert!(!verification.is_valid());
        assert!(verification.records().is_empty());
        assert!(
            verification
                .problem()
                .expect("problem")
                .message()
                .contains("seq is 2")
        );
    }

    #[test]
    fn a_malformed_line_is_reported_with_its_line_number() {
        let (_directory, log) = log();
        log.append_at(&entry("one"), at(0)).expect("one");
        let text = read_file(&log);
        write_file(&log, &format!("{text}{{\"not\":\"a record\"}}\n"));

        let error = log.read().expect_err("malformed");
        assert_eq!(error.code(), aicontext_core::ErrorCode::AUD_002);
        assert_eq!(error.line(), Some(2));
        let verification = log.verify();
        assert_eq!(verification.records().len(), 1, "the first record verified");
        assert_eq!(verification.problem().expect("problem").line(), Some(2));
    }

    #[test]
    fn a_record_with_an_unknown_field_is_malformed_rather_than_silently_ignored() {
        let (_directory, log) = log();
        log.append_at(&entry("one"), at(0)).expect("one");
        let mut value: Value = serde_json::from_str(lines(&log)[0].as_str()).expect("object");
        value["arguments"] = serde_json::json!({"secret": "value"});
        write_file(&log, &format!("{value}\n"));
        let error = log.read().expect_err("unknown field");
        assert_eq!(error.code(), aicontext_core::ErrorCode::AUD_002);
    }

    #[test]
    fn a_record_that_is_not_utf8_is_reported_as_malformed_not_as_empty() {
        let (_directory, log) = log();
        if let Some(parent) = log.path().parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        let mut file = fs::File::create(log.path()).expect("create");
        file.write_all(&[0xff, 0xfe, b'\n']).expect("write");
        let error = log.read().expect_err("invalid UTF-8");
        assert_eq!(error.code(), aicontext_core::ErrorCode::AUD_002);
    }

    #[test]
    fn a_log_over_the_cap_is_refused_without_parsing_it() {
        let (_directory, log) = log();
        if let Some(parent) = log.path().parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        let file = fs::File::create(log.path()).expect("create");
        file.set_len(MAX_LOG_BYTES + 1).expect("sets the length");
        let error = log.read().expect_err("over the cap");
        assert_eq!(error.code(), aicontext_core::ErrorCode::AUD_004);
        assert!(error.to_string().contains(&MAX_LOG_BYTES.to_string()));
    }

    #[test]
    fn appending_to_a_broken_chain_is_refused_and_writes_nothing() {
        let (_directory, log) = log();
        log.append_at(&entry("aws.ec2.stop"), at(0))
            .expect("append");
        let mut value: Value = serde_json::from_str(lines(&log)[0].as_str()).expect("object");
        value["action"]["tool"] = Value::String("aws.ec2.terminate".to_string());
        let broken = format!("{value}\n");
        write_file(&log, &broken);

        let error = log
            .append_at(&entry("aws.ec2.start"), at(1))
            .expect_err("refuses");
        assert_eq!(error.code(), aicontext_core::ErrorCode::AUD_005);
        assert!(error.to_string().contains("existing chain is not valid"));
        assert_eq!(read_file(&log), broken, "nothing was appended");
    }

    #[test]
    fn appending_to_a_malformed_log_is_refused_and_writes_nothing() {
        let (_directory, log) = log();
        write_file(&log, "{\n");
        let error = log.append_at(&entry("one"), at(0)).expect_err("refuses");
        assert_eq!(error.code(), aicontext_core::ErrorCode::AUD_005);
        assert_eq!(read_file(&log), "{\n");
    }

    #[test]
    fn a_blank_line_is_ignored_but_does_not_hide_a_bad_line_after_it() {
        let (_directory, log) = log();
        log.append_at(&entry("one"), at(0)).expect("one");
        let mut text = read_file(&log);
        text.push_str("\n\n{\n");
        write_file(&log, &text);
        let error = log.read().expect_err("the bad line is still found");
        assert_eq!(error.line(), Some(4));
    }

    #[test]
    fn crlf_line_endings_are_read_as_the_same_records() {
        let (_directory, log) = log();
        log.append_at(&entry("one"), at(0)).expect("one");
        let text = read_file(&log).replace('\n', "\r\n");
        write_file(&log, &text);
        assert_eq!(log.read().expect("reads").len(), 1);
    }

    #[test]
    fn the_append_refusal_keeps_the_underlying_cause_in_the_source_chain() {
        let (_directory, log) = log();
        write_file(&log, "not json\n");
        let error = log.append_at(&entry("one"), at(0)).expect_err("refuses");
        assert_eq!(error.code(), aicontext_core::ErrorCode::AUD_005);
        let source = std::error::Error::source(&error).expect("the cause survives");
        assert!(source.to_string().contains("not valid JSON"));
    }

    #[test]
    fn the_canonical_digest_is_stable_across_refactors() {
        // A chain that changes its digest scheme silently invalidates every existing log. This is the
        // one assertion that makes a change to canonicalisation a deliberate decision: the expected
        // value was produced by the documented scheme (sorted compact JSON without `self`), not by
        // copying the implementation's current output after the fact.
        let (_directory, log) = log();
        let record = log
            .append_at(&entry("aws.ec2.stop"), at(0))
            .expect("append");
        assert_eq!(
            record.digest(),
            "sha256:902af89c4ba891df533a0d81c017d9a747eddfe8a64e9d0a3df61cbf2e7451fa"
        );
    }

    #[test]
    fn append_creates_the_aicontext_directory() {
        let (_directory, log) = log();
        assert!(!log.path().parent().expect("parent").exists());
        log.append_at(&entry("one"), at(0)).expect("append");
        assert!(log.path().exists());
    }

    #[test]
    fn the_log_in_a_project_sits_under_aicontext() {
        let directory = TempDir::new().expect("temp dir");
        let log = AuditLog::in_project(directory.path());
        assert_eq!(
            log.path(),
            directory.path().join(".aicontext").join("audit.jsonl")
        );
    }

    #[test]
    fn a_verification_problem_is_a_copy_of_the_error_not_a_borrow_of_the_file() {
        let (_directory, log) = log();
        write_file(&log, "not json\n");
        let verification = log.verify();
        let problem = verification.problem().expect("problem").clone();
        assert_eq!(problem.code(), aicontext_core::ErrorCode::AUD_002);
        assert!(problem.message().contains("not valid JSON"));
        assert!(!problem.remediation().is_empty());
    }
}
