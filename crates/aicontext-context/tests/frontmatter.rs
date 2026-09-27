//! The front-matter parser, exercised through its public API.
//!
//! Split into three groups that mirror the acceptance criteria of `TASK-016`:
//!
//! 1. **Splitting and typing** - a leading block becomes a typed document, and the body is never
//!    interpreted.
//! 2. **Failure modes** - every malformed input produces a typed error with a line number, and
//!    never a panic.
//! 3. **Round-trips and properties** - what the parser reads, the renderer can write back, and
//!    that holds for documents nobody wrote by hand.
//!
//! Tests live out here rather than in the module so that they can only reach the public surface. If
//! something in this file compiles, it is API; if it does not, the API is incomplete.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use aicontext_context::{ContextError, Document, Value};

/// A representative document, using the shape from `docs/CONTEXT_SPEC.md` §2.
const TASK: &str = "\
---
id: TASK-014
type: task
title: Implement the lexical retriever
status: IN_PROGRESS
priority: HIGH
phase: 2
depends_on: [TASK-032]
created: 2026-09-27
updated: 2026-09-27
tags: [retrieval, context]
---

Human-readable body. Never parsed.
";

/// Wraps `block` in fences with `body` after it.
fn document(block: &str, body: &str) -> String {
    format!("---\n{block}---\n{body}")
}

// -- 0. The crate boundary --------------------------------------------------------

#[test]
fn the_yaml_library_stays_inside_the_codec() {
    // `ADR-007` accepts a dependency on a YAML library on the condition that nothing of it escapes:
    // `Value` is this crate's own type, and the codec is the only module that may know which library
    // is behind it. That is the whole mitigation for `RISK R-1`, and a promise nobody checks is not a
    // promise. So the source is scanned, rather than the public signatures, because a leak would
    // usually start as a private type used in a public one.
    let source_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut leaked = Vec::new();

    for entry in std::fs::read_dir(&source_dir).expect("the crate's own source is readable") {
        let path = entry.expect("a readable directory entry").path();
        if path.extension().and_then(std::ffi::OsStr::to_str) != Some("rs") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .expect("a name")
            .to_string();
        if name == "codec.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("a readable source file");
        if text.contains("yaml_serde") {
            leaked.push(name);
        }
    }

    assert!(
        leaked.is_empty(),
        "`yaml_serde` reached a module other than `codec.rs`: {leaked:?}. Put the conversion in the \
         codec and expose this crate's own `Value` instead, or revisit `ADR-007` first."
    );
}

// -- 1. Splitting and typing ----------------------------------------------------

#[test]
fn a_leading_block_becomes_a_typed_document() {
    let parsed = Document::parse(TASK).expect("the spec's own example must parse");
    let front_matter = parsed.front_matter().expect("front matter is present");

    assert_eq!(
        front_matter.id().map(ToString::to_string).as_deref(),
        Some("TASK-014")
    );
    assert_eq!(front_matter.kind(), Some("task"));
    assert_eq!(
        front_matter.title(),
        Some("Implement the lexical retriever")
    );
    assert_eq!(front_matter.status(), Some("IN_PROGRESS"));
    assert_eq!(front_matter.created(), Some("2026-09-27"));
    assert_eq!(front_matter.updated(), Some("2026-09-27"));
    assert_eq!(front_matter.tags(), ["retrieval", "context"]);
}

#[test]
fn a_type_specific_key_is_preserved_as_a_value_rather_than_guessed_at() {
    let parsed = Document::parse(TASK).expect("parses");
    let front_matter = parsed.front_matter().expect("front matter is present");

    // `phase` and `depends_on` belong to a task, and `priority` to this document type. Which of
    // them are legal depends on the schema, which is TASK-015's business, so they are kept intact
    // and interpreted by nobody here.
    //
    // Each is checked through an accessor rather than against a constructed `Value`, because
    // `Value` is `#[non_exhaustive]`. That reads better anyway: the assertion states the expected
    // *type* as well as the expected content, which a whole-value comparison would not.
    assert_eq!(front_matter.extra("phase").and_then(Value::as_int), Some(2));
    assert_eq!(
        front_matter.extra("priority").and_then(Value::as_str),
        Some("HIGH")
    );
    assert_eq!(
        front_matter
            .extra("depends_on")
            .and_then(Value::as_list)
            .map(<[Value]>::len),
        Some(1),
        "`depends_on` is a list, not a string"
    );
    assert_eq!(
        front_matter
            .extra("depends_on")
            .and_then(Value::as_list)
            .and_then(|items| items.first())
            .and_then(Value::as_str),
        Some("TASK-032"),
        "and its one item is the string that was written"
    );
}

#[test]
fn the_body_is_returned_untouched_and_never_parsed() {
    // A body that would break any Markdown-aware reader: a heading, a horizontal rule, a fence, a
    // table, and front matter that looks like a second block. None of it may change the result.
    let body = "\
# A heading

---

```yaml
id: NOT-A-DOCUMENT
```

| a | b |
|---|---|
| 1 | 2 |

---
id: ALSO-NOT-A-DOCUMENT
";
    let parsed =
        Document::parse(&document("id: TASK-014\ntype: task\ntitle: T\n", body)).expect("parses");

    assert_eq!(parsed.body(), body, "the body must come back byte for byte");
    assert_eq!(
        parsed
            .front_matter()
            .and_then(|front| front.id())
            .map(ToString::to_string),
        Some("TASK-014".to_string()),
        "a body that looks like front matter is still just body"
    );
}

#[test]
fn a_document_with_no_block_is_all_body() {
    let source = "# AI.md\n\nNo front matter here.\n";
    let parsed = Document::parse(source).expect("a document without a block is legal");

    assert!(
        parsed.front_matter().is_none(),
        "no fence means no front matter, which is different from an empty block"
    );
    assert_eq!(parsed.body(), source);
}

#[test]
fn an_empty_block_is_not_the_same_as_no_block() {
    // `---` twice is something a human writes. It is a block with no fields, and the missing
    // id/type/title are the schema's finding to raise, not the parser's.
    let parsed = Document::parse("---\n---\nbody\n").expect("an empty block is not a syntax error");

    let front_matter = parsed
        .front_matter()
        .expect("a block is present, however empty");
    assert_eq!(
        front_matter.missing_required_keys(),
        ["id", "type", "title"]
    );
    assert_eq!(parsed.body(), "body\n");
}

#[test]
fn missing_required_keys_are_named_in_the_documented_order() {
    let parsed = Document::parse(&document("type: task\n", "")).expect("parses");
    let front_matter = parsed.front_matter().expect("front matter is present");

    assert_eq!(front_matter.missing_required_keys(), ["id", "title"]);
}

#[test]
fn a_complete_block_reports_nothing_missing() {
    let parsed = Document::parse(TASK).expect("parses");
    let front_matter = parsed.front_matter().expect("front matter is present");

    assert!(front_matter.missing_required_keys().is_empty());
}

#[test]
fn a_windows_authored_file_parses_identically() {
    let unix = Document::parse(TASK).expect("parses");
    let windows = Document::parse(&TASK.replace('\n', "\r\n")).expect("CRLF must parse too");

    assert_eq!(
        unix.front_matter(),
        windows.front_matter(),
        "line endings are an editor's business, not the document's"
    );
}

#[test]
fn a_fence_with_trailing_space_is_still_a_fence() {
    let parsed = Document::parse("---\nid: TASK-014\n---  \nbody\n").expect("parses");

    assert!(parsed.front_matter().is_some());
    assert_eq!(parsed.body(), "body\n");
}

// -- 2. Failure modes ------------------------------------------------------------

#[test]
fn an_unterminated_block_names_the_line_it_opened_on() {
    let error = Document::parse("---\nid: TASK-014\ntype: task\n").expect_err("never closed");

    assert_eq!(
        error.line(),
        Some(1),
        "the line the author has to fix is where the block opened"
    );
    assert!(matches!(
        error,
        ContextError::FrontMatterNotClosed { line: 1 }
    ));
}

#[test]
fn a_yaml_syntax_error_names_the_offending_line_of_the_file() {
    // The block's second line is file line 3. Getting this arithmetic wrong is the failure this
    // test exists to catch: a number a line off sends the author to the wrong place, and the
    // block-relative number is never what they see in their editor.
    let source = "---\nid: TASK-014\ntitle: a: b\n---\nbody\n";
    let error = Document::parse(source).expect_err("a mapping value is not allowed there");

    assert_eq!(error.line(), Some(3), "got: {error}");
    assert!(
        error.to_string().starts_with("line 3: "),
        "the file's line must lead the message, got: {error}"
    );
}

#[test]
fn a_repeated_key_is_refused_and_named() {
    // The alternative, which most YAML libraries choose silently, is to keep the last value and
    // lose the first. A storage contract may not do that.
    let error = Document::parse(&document("id: TASK-014\nid: TASK-015\n", "")).expect_err("dup");

    assert!(
        matches!(&error, ContextError::DuplicateKey { key, .. } if key == "id"),
        "the error must name the repeated key, got: {error}"
    );
    assert_eq!(error.line(), Some(3), "the second occurrence is the error");
}

#[test]
fn a_repeated_nested_key_is_refused_too() {
    let error = Document::parse(&document("outer: {a: 1, a: 2}\n", "")).expect_err("dup");

    assert!(
        error.to_string().to_lowercase().contains("duplicate"),
        "got: {error}"
    );
}

#[test]
fn a_non_mapping_block_is_refused() {
    let error = Document::parse(&document("- one\n- two\n", "")).expect_err("a list is not fields");

    assert_eq!(error.line(), Some(2), "got: {error}");
    assert!(
        error.to_string().contains("found a list"),
        "must say what it found, so the author knows it is the shape and not the syntax: {error}"
    );
}

#[test]
fn a_badly_typed_known_key_is_refused_with_the_shape_it_found() {
    let error = Document::parse(&document(
        "id: TASK-014\ntype: task\ntitle: T\ntags: retrieval\n",
        "",
    ))
    .expect_err("tags must be a list");

    assert_eq!(error.line(), Some(5), "got: {error}");
    let message = error.to_string();
    assert!(message.contains("`tags`"), "must name the key: {message}");
    assert!(
        message.contains("a list of strings") && message.contains("a string"),
        "must say what was expected and what was found: {message}"
    );
}

#[test]
fn a_list_containing_a_non_string_is_refused() {
    let error = Document::parse(&document(
        "id: TASK-014\ntype: task\ntitle: T\ntags: [a, 2]\n",
        "",
    ))
    .expect_err("every tag must be a string");

    assert!(error.to_string().contains("an integer"), "got: {error}");
}

#[test]
fn a_key_holding_the_wrong_shape_is_refused_rather_than_coerced() {
    let error = Document::parse(&document("id: 14\ntype: task\ntitle: T\n", ""))
        .expect_err("an id is not a number");

    assert!(
        error.to_string().contains("must be a string"),
        "got: {error}"
    );
    assert!(error.to_string().contains("an integer"), "got: {error}");
}

#[test]
fn a_malformed_identifier_is_refused_with_the_identifiers_own_reason() {
    let error = Document::parse(&document("id: task-014\ntype: task\ntitle: T\n", ""))
        .expect_err("identifiers are uppercase");

    assert!(
        matches!(error, ContextError::InvalidId { .. }),
        "got: {error}"
    );
    assert!(error.to_string().contains("task-014"), "got: {error}");
    assert_eq!(error.line(), Some(2), "got: {error}");
}

#[test]
fn an_impossible_date_is_refused() {
    let cases = [
        "created: 2026-02-31",
        "created: 2026-13-01",
        "created: 27-09-2026",
        "created: yesterday",
    ];

    for case in cases {
        let error = Document::parse(&document(
            &format!("id: TASK-014\ntype: task\ntitle: T\n{case}\n"),
            "",
        ))
        .expect_err("a date must be a real day");
        assert!(
            matches!(error, ContextError::InvalidDate { .. }),
            "{case} should be refused as a date, got: {error}"
        );
    }
}

#[test]
fn a_leap_day_is_accepted_because_it_is_a_real_day() {
    let parsed = Document::parse(&document(
        "id: TASK-014\ntype: task\ntitle: T\ncreated: 2024-02-29\n",
        "",
    ))
    .expect("2024-02-29 is a real date");

    assert_eq!(
        parsed.front_matter().and_then(|front| front.created()),
        Some("2024-02-29")
    );
}

#[test]
fn a_tagged_value_is_refused_rather_than_half_understood() {
    let error = Document::parse(&document("id: !Custom TASK-014\n", "")).expect_err("a tag");

    assert!(error.to_string().contains("!Custom"), "got: {error}");
}

#[test]
fn an_oversized_document_is_refused_rather_than_read() {
    // RULES.md 11: no unbounded reads. The limit has to stop the read, not be noted afterwards.
    let huge = "x".repeat(aicontext_context::DOCUMENT_MAX_BYTES + 1);
    let error = Document::parse(&huge).expect_err("over the document limit");

    assert!(
        matches!(error, ContextError::DocumentTooLarge { .. }),
        "got: {error}"
    );
    assert!(
        error.to_string().contains("1048576"),
        "must state the limit: {error}"
    );
}

#[test]
fn an_oversized_block_is_refused_even_in_a_small_document() {
    // The padding has to be a *line*, or the closing fence would be glued onto it and the block
    // would look unterminated instead of oversized.
    let padding = format!(
        "{}\n",
        "x".repeat(aicontext_context::FRONT_MATTER_MAX_BYTES + 1)
    );
    let error = Document::parse(&document(&padding, "")).expect_err("over the front-matter limit");

    assert!(
        matches!(error, ContextError::FrontMatterTooLarge { .. }),
        "got: {error}"
    );
    assert!(
        error.to_string().contains("65536"),
        "must state the limit: {error}"
    );
}

#[test]
fn every_failure_tells_the_author_what_to_do() {
    // A parser error with no remediation sends the reader to the source instead.
    let cases = [
        "---\nid: TASK-014\n",
        "---\nid: [1\n---\n",
        "---\nid: TASK-014\nid: TASK-015\n---\n",
        "---\nid: 14\n---\n",
        "---\nid: task-014\n---\n",
        "---\ncreated: 2026-02-31\n---\n",
        "---\ntags: nope\n---\n",
        "---\n- a\n- b\n---\n",
    ];

    for case in cases {
        let error = Document::parse(case).expect_err("must be refused");
        let converted: aicontext_core::AicontextError = error.into();
        assert!(
            !converted.remediation().is_empty(),
            "no remediation for {case:?}: {converted}"
        );
        assert!(
            converted.message().contains("line")
                || converted.code() == aicontext_core::ErrorCode::CTX_018,
            "no line in the message for {case:?}: {converted}"
        );
    }
}

#[test]
fn no_malformed_input_ever_panics() {
    // A parser that panics on a bad file is a denial-of-service bug in a tool whose whole job is
    // reading files someone else wrote. Every one of these is hostile input.
    let hostile = [
        "",
        "-",
        "---",
        "----",
        "\n---\nid: TASK-014\n---\n",
        "---\n---\n---\n",
        "---\n\tid: TASK-014\n---\n",
        "---\nid: \"unterminated\n---\n",
        "---\nid: TASK-014\n  \tbad: [}\n---\n",
        "---\n\u{feff}id: TASK-014\n---\n",
        "---\nid: \u{202e}TASK-014\n---\n",
        "---\n\0\n---\n",
        "---\n\"\\u0000\": 1\n---\n",
        "---\nid: |\n  block\n   scalar\n---\n",
        "---\nid: >\n  folded\n---\n",
        "---\n? complex\n: key\n---\n",
        "---\nid: TASK-014\n...\n---\n",
    ];

    for case in hostile {
        // The assertion is that we get here at all.
        let _ = Document::parse(case);
    }
}

// -- 3. Round-trips --------------------------------------------------------------

#[test]
fn the_specs_own_example_round_trips_in_value_even_though_it_is_rewritten() {
    // The example in CONTEXT_SPEC.md 2 uses flow-style lists and puts its type-specific keys in an
    // order of the author's choosing. A rewrite normalises both, so it does not come back byte for
    // byte - which is the documented behaviour, and the reason the round-trip property is stated
    // over values rather than over text.
    let parsed = Document::parse(TASK).expect("the spec's own example must parse");
    let rendered = parsed
        .render()
        .expect("a parsed document is always writable");
    let reparsed = Document::parse(&rendered).expect("what we write, we can read");

    assert_eq!(
        parsed.front_matter(),
        reparsed.front_matter(),
        "no value may change\nrendered:\n{rendered}"
    );
    assert_eq!(
        reparsed
            .render()
            .expect("a parsed document is always writable"),
        rendered,
        "rendering is not a fixed point"
    );
    assert!(
        rendered.contains("depends_on:\n- TASK-032"),
        "a list survives:\n{rendered}"
    );
}

#[test]
fn a_canonical_document_renders_back_to_itself_byte_for_byte() {
    // What a tool-written file looks like: one key per line, no flow style. This is the case where
    // rendering is exactly reversible, and it is the case that matters for `aicontext edit`.
    let canonical = "---\n\
                     id: TASK-014\n\
                     type: task\n\
                     title: Implement the lexical retriever\n\
                     status: IN_PROGRESS\n\
                     phase: 2\n\
                     priority: HIGH\n\
                     created: 2026-09-27\n\
                     updated: 2026-09-27\n\
                     tags:\n\
                     - retrieval\n\
                     - context\n\
                     ---\n\
                     \n\
                     Human-readable body. Never parsed.\n";

    assert_eq!(
        Document::parse(canonical)
            .expect("parses")
            .render()
            .expect("a parsed document is always writable"),
        canonical
    );
}

#[test]
fn a_document_with_no_block_renders_back_to_itself() {
    let source = "# AI.md\n\nNothing but prose.\n";
    assert_eq!(
        Document::parse(source)
            .expect("parses")
            .render()
            .expect("a parsed document is always writable"),
        source
    );
}

#[test]
fn rendering_puts_the_well_known_keys_in_the_documented_order() {
    // CONTEXT_SPEC.md 2 rule 4: id, type, title, status, type-specific, created, updated, tags.
    let parsed = Document::parse(
        "---\ntags: [a]\nupdated: 2026-09-27\nphase: 2\nstatus: DONE\ntitle: T\n\
         created: 2026-09-27\ntype: task\nid: TASK-014\n---\nbody\n",
    )
    .expect("parses");

    let rendered = parsed
        .render()
        .expect("a parsed document is always writable");
    // Only the lines that are a key at column 0. A block-style list puts its items on their own
    // indented lines, and counting those as keys is how this test would have passed on the wrong
    // answer.
    let order: Vec<&str> = rendered
        .lines()
        .skip(1)
        .take_while(|line| *line != "---")
        .filter(|line| !line.starts_with([' ', '-', '\t']))
        .filter_map(|line| line.split(':').next())
        .collect();

    assert_eq!(
        order,
        [
            "id", "type", "title", "status", "phase", "created", "updated", "tags"
        ],
        "a rewrite must normalise to the documented order, got:\n{rendered}"
    );
}

#[test]
fn rendering_canonicalises_key_order_and_is_then_a_fixed_point() {
    let messy = "---\nzeta: 1\nalpha: 2\nid: TASK-014\n---\nbody\n";
    let once = Document::parse(messy)
        .expect("parses")
        .render()
        .expect("a parsed document is always writable");
    let twice = Document::parse(&once)
        .expect("the output must re-parse")
        .render()
        .expect("a parsed document is always writable");

    assert_eq!(once, twice, "rendering must be idempotent");
    assert!(
        once.contains("alpha: 2") && once.contains("zeta: 1"),
        "no key may be dropped"
    );
}

#[test]
fn a_value_that_looks_like_the_format_survives_a_round_trip() {
    // The nastiest realistic case: a human pastes an example block into a title.
    let awkward = "id: TASK-014\ntype: task\ntitle: \"a: b # c\"\nstatus: IN_PROGRESS\n";

    let source = document(awkward, "body\n");
    let parsed = Document::parse(&source).expect("parses");
    let rendered = parsed
        .render()
        .expect("a parsed document is always writable");
    let reparsed = Document::parse(&rendered).expect("what we write, we can read");

    assert_eq!(
        parsed.front_matter(),
        reparsed.front_matter(),
        "rendered:\n{rendered}"
    );
}

#[test]
fn an_empty_body_survives_a_round_trip() {
    let source = document("id: TASK-014\n", "");
    let parsed = Document::parse(&source).expect("parses");

    assert_eq!(
        parsed
            .render()
            .expect("a parsed document is always writable"),
        source
    );
    assert_eq!(parsed.body(), "");
}

#[test]
fn an_empty_tag_list_is_not_the_same_fact_as_no_tag_key() {
    // `tags: []` says the document has no tags; an absent `tags` says nobody has said anything
    // about its tags. A rewrite that collapsed them would quietly delete a key the author wrote.
    let written = document("id: TASK-014\ntags: []\n", "body\n");
    let absent = document("id: TASK-014\n", "body\n");

    let parsed_written = Document::parse(&written).expect("parses");
    let parsed_absent = Document::parse(&absent).expect("parses");

    assert_eq!(
        parsed_written.front_matter().expect("has one").tags(),
        Vec::<String>::new()
    );
    assert_ne!(
        parsed_written.front_matter(),
        parsed_absent.front_matter(),
        "the two documents differ in a fact, so they are not equal"
    );

    let rendered = parsed_written
        .render()
        .expect("a parsed document is always writable");
    assert!(
        rendered.contains("tags: []"),
        "the key the author wrote must still be there, got:\n{rendered}"
    );
    assert_eq!(rendered, written, "and nothing else may change either");

    assert_eq!(
        parsed_absent
            .render()
            .expect("a parsed document is always writable"),
        absent,
        "a document without the key must not acquire one"
    );
}

#[test]
fn keys_the_parser_does_not_interpret_are_preserved_exactly() {
    // Rule 5: an unknown key is a warning, never a data loss, so that a newer schema does not
    // break an older binary. It has to come back out.
    let source = document(
        "id: TASK-014\ntype: task\ntitle: T\nfuture_key: {nested: [1, true, null]}\n",
        "body\n",
    );
    let parsed = Document::parse(&source).expect("parses");
    let reparsed = Document::parse(
        &parsed
            .render()
            .expect("a parsed document is always writable"),
    )
    .expect("re-parses");

    let nested = reparsed
        .front_matter()
        .and_then(|front| front.extra("future_key"))
        .expect("the unknown key must survive");
    let items = nested
        .get("nested")
        .and_then(Value::as_list)
        .expect("a mapping is preserved as a mapping, nesting intact");
    // Checked one item at a time through the accessors, for the same reason as
    // `a_type_specific_key_is_preserved_as_a_value_rather_than_guessed_at`: a `Value` cannot be
    // constructed from out here, and the type of each item is half of what is being asserted.
    assert_eq!(items.len(), 3, "the sequence kept its length");
    assert_eq!(items[0].as_int(), Some(1));
    assert_eq!(items[1].as_bool(), Some(true));
    assert_eq!(items[2].type_name(), "null");
}

#[test]
fn a_float_keeps_a_decimal_point_so_it_does_not_come_back_as_an_integer() {
    // The trap this guards: `1.0` printed without a decimal point is `1`, and YAML reads `1` as an
    // integer. A front-matter value that silently changes type across a rewrite would break any
    // future schema that cares whether a number is whole. The renderer keeps the point, and this
    // pins that it does, including for the whole-valued case and the negative one.
    for (written, expected) in [
        ("1.0", "a float"),
        ("1.5", "a float"),
        ("0.1", "a float"),
        ("-2.25", "a float"),
    ] {
        let source = document(
            &format!("id: TASK-014\ntype: task\ntitle: T\nweight: {written}\n"),
            "body\n",
        );
        let parsed = Document::parse(&source).expect("a float is a valid value");
        assert_eq!(
            parsed
                .front_matter()
                .and_then(|front| front.extra("weight"))
                .map(Value::type_name),
            Some(expected),
            "`{written}` must be read as a float"
        );
        assert_eq!(
            parsed
                .render()
                .expect("a parsed document is always writable")
                .lines()
                .find(|line| line.starts_with("weight:"))
                .expect("the key is still there"),
            format!("weight: {written}"),
            "`{written}` must be written back unchanged"
        );
    }
}

// -- 4. Properties ---------------------------------------------------------------

use proptest::prelude::*;

/// Quotes a string as a YAML double-quoted scalar.
///
/// The generators below need to put arbitrary text into a block, and Rust's `{:?}` is not a YAML
/// quoter: it writes `\u{e000}` where YAML requires `\uE000`, so a test using it would be
/// measuring the test's own escaping rather than the parser. Only the five characters that would
/// otherwise end or continue the scalar are escaped, and everything else - including every
/// non-ASCII character - is left literal, which is what a human writing YAML actually does.
fn yaml_quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for character in text.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

/// A value shape a fixture can hold, in the terms this file chose.
///
/// Deliberately its own type rather than the crate's `Value`. `Value` is `#[non_exhaustive]`, so a
/// test could not construct one anyway, and the round-trip property would be stronger for it: the
/// fixture is now described in a vocabulary the parser has never seen, so a fixture that comes back
/// intact is evidence about the *format* rather than agreement between the crate's generator and the
/// crate's renderer.
#[derive(Clone, Debug)]
enum Shape {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Shape>),
    Map(BTreeMap<String, Shape>),
}

/// Emits a value as YAML block style, at the given indentation.
///
/// Independent of the crate's own renderer on purpose. Using `Document::render` to build the
/// fixtures would make the round-trip property circular: it would only prove that the renderer
/// agrees with itself.
fn entry(key: &str, value: &Shape, indent: usize) -> String {
    let pad = " ".repeat(indent);
    match value {
        Shape::List(items) if !items.is_empty() => {
            let mut out = format!("{pad}{key}:\n");
            for item in items {
                out.push_str(&sequence_item(item, indent + 2));
            }
            out
        }
        Shape::Map(nested) if !nested.is_empty() => {
            let mut out = format!("{pad}{key}:\n");
            for (nested_key, nested_value) in nested {
                out.push_str(&entry(nested_key, nested_value, indent + 2));
            }
            out
        }
        scalar => format!("{pad}{key}: {}\n", scalar_text(scalar)),
    }
}

/// A `- item` entry at the given indentation, including the trailing newline.
fn sequence_item(value: &Shape, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let nested: String = match value {
        Shape::List(items) if !items.is_empty() => items
            .iter()
            .map(|item| sequence_item(item, indent + 2))
            .collect(),
        Shape::Map(entries) if !entries.is_empty() => entries
            .iter()
            .map(|(key, item)| entry(key, item, indent + 2))
            .collect(),
        scalar => return format!("{pad}- {}\n", scalar_text(scalar)),
    };

    // The first nested line rides on the dash line; the rest line up two columns further along,
    // which is where the nested block already puts them. The trailing newline of the nested block
    // is re-added rather than inherited, because `lines()` has just taken it off.
    let nested = nested.trim_end_matches('\n');
    let mut lines = nested.split('\n');
    let first = lines.next().unwrap_or_default().trim_start();
    let mut out = format!("{pad}- {first}\n");
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// A scalar in YAML syntax, or the inline form of an empty collection.
fn scalar_text(value: &Shape) -> String {
    match value {
        Shape::Null => "null".to_string(),
        Shape::Bool(inner) => inner.to_string(),
        Shape::Int(inner) => inner.to_string(),
        Shape::Float(inner) => {
            // `{}` prints 1.0f64 as `1`, which YAML would read back as an integer, so the fixture
            // would not hold the float it claims to. Give a whole value a decimal point, which is
            // also what the renderer does.
            let text = inner.to_string();
            if text.contains('.') {
                text
            } else {
                format!("{text}.0")
            }
        }
        Shape::Str(inner) => yaml_quote(inner),
        Shape::List(_) => "[]".to_string(),
        Shape::Map(_) => "{}".to_string(),
    }
}

/// Arbitrary text, kept short enough that the generator, not the parser, is the slow part.
///
/// Unlike [`writable_text`] this makes no attempt to produce something YAML can hold: the point is
/// to reach the parser with bytes no human would write, including control characters and lone
/// non-characters, and require that they are refused rather than crashed on.
fn arbitrary_text() -> impl Strategy<Value = String> {
    any::<String>().prop_map(|text: String| text.chars().take(512).collect())
}

/// Text a human can actually put in a front-matter file.
///
/// YAML 1.2's printable set excludes the control blocks and the Unicode non-characters, and a
/// double-quoted scalar holding one is not a document anyone could write - the layer refuses it as a
/// control character, whether it is a C0 control, a C1 control, or `U+FFFE`. Excluding them here is
/// a statement about the format, not a way of making a test pass: `arbitrary_text_is_never_a_panic`
/// still feeds the parser exactly this text and checks that it is refused cleanly.
fn writable_text() -> impl Strategy<Value = String> {
    any::<String>()
        .prop_map(|text: String| text.chars().take(120).collect())
        .prop_filter(
            "YAML has no spelling for a control character or a non-character",
            |text: &String| text.chars().all(is_writable),
        )
}

/// Whether a character may appear in a YAML double-quoted scalar in front matter.
///
/// `char::is_control` is Unicode `Cc`, which is exactly the C0 block, `DEL`, and the C1 block, so the
/// non-characters are the only thing left among those. There is no `char::is_noncharacter` in the
/// standard library, and there are 34 of them across 17 planes, so they are named by their low
/// sixteen bits.
///
/// The four line breaks are here for a different reason: YAML ends a line on all of them, while a
/// file's lines are separated by LF alone, so front matter refuses them (see the parser's
/// `first_foreign_line_break`). A quoted scalar holding a bare LS would be folded across two lines
/// by the YAML layer, so the character could not survive the round-trip in any case.
fn is_writable(character: char) -> bool {
    if character.is_control() {
        return false;
    }
    let low = character as u32 & 0xFFFF;
    !matches!(low, 0xFFFE | 0xFFFF)
        && !('\u{fdd0}'..='\u{fdef}').contains(&character)
        && !matches!(character, '\r' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

mod strategy {
    use super::Shape;
    use super::writable_text;
    use proptest::prelude::*;

    /// A key that a human could plausibly have written.
    pub(crate) fn key() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_]{0,12}".prop_map(String::from)
    }

    /// A float that both this file and the renderer can spell the same way.
    ///
    /// Three things are dodged on purpose. The whole part is a `u32`, because `f64::from(u32)` is
    /// exact and so the value is a real decimal with a short expansion, whereas a bare `as f64` from
    /// a wider integer would be the kind of quiet imprecision this crate refuses to store. The
    /// fraction comes from a fixed set rather than a raw float range, which keeps the expansion
    /// short, and the sign is applied to a non-zero magnitude, so no negative zero appears.
    pub(crate) fn float() -> impl Strategy<Value = f64> {
        (
            prop::sample::select(vec![-1.0f64, 1.0]),
            0u32..10_000,
            prop::sample::select(vec![0.0f64, 0.001, 0.125, 0.25, 0.5, 0.999]),
        )
            .prop_map(|(sign, whole, fraction)| sign * (f64::from(whole) + fraction))
    }

    /// A scalar, weighted towards the shapes front matter actually holds.
    ///
    /// Strings go through [`writable_text`] because a value that goes into a block has to be one a
    /// file can hold: an arbitrary string would put a control character or a bare line separator in
    /// the fixture, and the round-trip property would then be asserting that the parser refuses it.
    pub(crate) fn scalar() -> impl Strategy<Value = Shape> {
        prop_oneof![
            writable_text().prop_map(Shape::Str),
            any::<bool>().prop_map(Shape::Bool),
            (-1_000_000i64..1_000_000).prop_map(Shape::Int),
            float().prop_map(Shape::Float),
            Just(Shape::Null),
        ]
    }

    /// Any value, nested to a shallow depth so the generator stays fast and terminating.
    pub(crate) fn value() -> impl Strategy<Value = Shape> {
        scalar().prop_recursive(2, 8, 2, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..4).prop_map(Shape::List),
                prop::collection::btree_map(key(), inner, 0..4).prop_map(Shape::Map),
            ]
        })
    }
}

/// A block of type-specific keys, none of them a well-known one, so the parser cannot be
/// interpreting any of them.
fn extras() -> impl Strategy<Value = Vec<(String, Shape)>> {
    prop::collection::btree_map(
        strategy::key().prop_filter("not a well-known key", |key| {
            !matches!(
                key.as_str(),
                "id" | "type" | "title" | "status" | "created" | "updated" | "tags"
            )
        }),
        strategy::value(),
        0..4,
    )
    .prop_map(|entries| entries.into_iter().collect())
}

proptest! {
    /// Whatever the parser reads, it can write back, and what it writes re-reads to the same thing.
    ///
    /// This is the property that lets `aicontext` rewrite a document without corrupting it, which
    /// is the whole reason the renderer exists.
    #[test]
    fn any_front_matter_round_trips(
        // The prefix is 2 to 8 uppercase letters and the suffix is digits, because that is the
        // grammar `DocumentId` enforces. A generator wider than the grammar would only be measuring
        // the identifier's own validation, which has its own tests.
        id in proptest::option::of("[A-Z]{2,8}-[0-9]{1,4}"),
        title in proptest::option::of(writable_text()),
        status in proptest::option::of("[A-Z_]+"),
        tags in prop::collection::vec("[a-z]{1,8}", 0..4),
        extra in extras(),
        body in ".{0,60}",
    ) {
        let mut block = String::new();
        // The renderer emits the extras sorted, because the format prescribes no order for them.
        let mut extra = extra;
        extra.sort_by(|left, right| left.0.cmp(&right.0));

        if let Some(id) = id.as_ref() {
            let _ = writeln!(block, "id: {id}");
        }
        if let Some(title) = title.as_ref() {
            block.push_str(&entry("title", &Shape::Str(title.clone()), 0));
        }
        if let Some(status) = status.as_ref() {
            let _ = writeln!(block, "status: {status}");
        }
        for (key, value) in &extra {
            block.push_str(&entry(key, value, 0));
        }
        if !tags.is_empty() {
            block.push_str("tags:\n");
            for tag in &tags {
                let _ = writeln!(block, "- {tag}");
            }
        }

        let source = format!("---\n{block}---\n{body}");
        let first = Document::parse(&source).map_err(|error|
            TestCaseError::fail(format!("could not parse a conforming block: {error}\n{block}")))?;

        let rendered = first.render().expect("a parsed document is always writable");
        let second = Document::parse(&rendered).map_err(|error|
            TestCaseError::fail(format!("could not re-parse our own output: {error}\n{rendered}")))?;

        prop_assert_eq!(
            &first.front_matter(),
            &second.front_matter(),
            "round trip changed the document\nsource:\n{}\nrendered:\n{}",
            source,
            rendered
        );
        prop_assert_eq!(&second.render().expect("a parsed document is always writable"), &rendered, "rendering is not a fixed point");
        prop_assert_eq!(second.body(), body.as_str(), "the body was not preserved");
    }

    /// A title is a human string, so it may contain anything at all - quotes, colons, newlines,
    /// backslashes, emoji, and the fence itself. It must always come back unchanged.
    #[test]
    fn any_title_survives_unchanged(title in writable_text()) {
        let source = format!("---\ntitle: {}\n---\nbody\n", yaml_quote(&title));
        let parsed = Document::parse(&source)
            .map_err(|error| TestCaseError::fail(format!("could not parse: {error}\n{source}")))?;

        let rendered = parsed.render().expect("a parsed document is always writable");
        let reparsed = Document::parse(&rendered)
            .map_err(|error| TestCaseError::fail(format!("could not re-parse: {error}\n{rendered}")))?;

        prop_assert_eq!(
            reparsed.front_matter().and_then(|front| front.title()),
            Some(title.as_str()),
            "the title changed\nsource:\n{}\nrendered:\n{}",
            source,
            rendered
        );
    }

    /// A document whose body is arbitrary text is either a body or a block, and never a panic,
    /// whatever it contains.
    #[test]
    fn any_body_is_never_interpreted(body in ".{0,200}") {
        let source = format!("---\nid: TASK-014\ntype: task\ntitle: T\n---\n{body}");
        let parsed = Document::parse(&source)
            .map_err(|error| TestCaseError::fail(format!("could not parse: {error}")))?;

        prop_assert_eq!(parsed.body(), body.as_str());
    }

    /// The fence is only a fence on the first line, so a document whose body opens with a
    /// horizontal rule is a document whose rule is body text.
    #[test]
    fn a_rule_after_the_block_stays_in_the_body(body in ".{0,80}") {
        let source = format!("---\nid: TASK-014\n---\n--- {body}\n");
        let parsed = Document::parse(&source).expect("parses");

        prop_assert_eq!(parsed.body(), format!("--- {body}\n"));
        prop_assert!(parsed.front_matter().is_some());
    }

    /// Nothing a file can contain may make the parser panic.
    ///
    /// Acceptance for `TASK-016` is that a bad document produces a typed error, and the other half of
    /// that promise is that no input produces a crash. Arbitrary text mostly misses the front-matter
    /// path, so the block-shaped half of this property does the real work: a fence, then bytes that
    /// are only *nearly* YAML, then a closing fence.
    #[test]
    fn arbitrary_text_is_never_a_panic(source in arbitrary_text()) {
        let outcome = std::panic::catch_unwind(|| Document::parse(&source));
        prop_assert!(
            outcome.is_ok(),
            "the parser panicked on:\n{:?}\nsource:\n{source:?}",
            outcome.err().map(|_| "unwind"),
        );
    }

    /// The same, for input shaped like a document, where the YAML layer actually runs.
    #[test]
    fn arbitrary_block_contents_are_never_a_panic(
        block in arbitrary_text(),
        body in ".*",
        closing in prop::sample::select(vec!["---", "--- \n", "--- # note", ""]),
    ) {
        let source = format!("---\n{block}{closing}\n{body}");
        let outcome = std::panic::catch_unwind(|| Document::parse(&source));

        prop_assert!(
            outcome.is_ok(),
            "the parser panicked on a block of:\n{block:?}\nclosing:\n{closing:?}"
        );
    }

    /// A reported line is never zero, never negative, and never past the end of the file.
    ///
    /// The error type's whole argument is that the number is the first thing an author uses, and a
    /// number pointing past the end of the file is the failure mode of the block-to-file offset
    /// arithmetic. A position one line *past* the end is allowed, because a scanner that runs out of
    /// input reports where it stopped noticing, which is the line after the last one.
    #[test]
    fn a_reported_failure_line_points_somewhere_real(
        block in prop::collection::vec("[a-z]{1,6}: .*", 1..6),
    ) {
        let source = format!("---\n{}\n---\n", block.join("\n"));
        let Err(error) = Document::parse(&source) else {
            return Ok(());
        };
        let Some(line) = error.line() else {
            return Ok(());
        };
        let lines = source.lines().count();

        prop_assert!(line >= 1, "a line number is 1-based, got {line}");
        prop_assert!(
            line <= lines + 1,
            "line {line} is outside a file of {lines} lines:\n{source}"
        );
    }
}
