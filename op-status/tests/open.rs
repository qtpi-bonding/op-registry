#![allow(clippy::err_expect, clippy::ok_expect)]
mod common;

use coded_error::ErrorClass;
use common::*;
use futures_util::StreamExt;
use op_status::{
    open_status, Component, OpenedStatus, StatusCursorRef, StatusFilter, StatusSource, WATCH_BUFFER,
};

async fn open<'a, S: StatusSource + ?Sized>(
    src: &'a S,
    limit: usize,
    cursor: Option<StatusCursorRef>,
) -> anyhow::Result<OpenedStatus<'a>> {
    open_status(src, &StatusFilter::default(), limit, cursor).await
}

fn cur(epoch: u64, seq: u64) -> Option<StatusCursorRef> {
    Some(StatusCursorRef { epoch, seq })
}

async fn drain(opened: OpenedStatus<'_>) -> Vec<anyhow::Result<(u64, Component)>> {
    opened.changes.collect().await
}

/// Fills the log with `n` ready changes after seq `after` and sets newest = after.
fn fill<const W: bool>(src: &ToyW<W>, after: u64, n: u64) {
    for i in 1..=n {
        src.push(after + i, comp(&format!("c{i}")));
    }
    src.set_window(1, after);
}

#[tokio::test]
async fn open_status_without_a_cursor_is_a_full_snapshot_with_resync_true() {
    let src = Toy::new().with_rows(vec![comp("a"), comp("b")]);
    src.set_window(1, 3);
    let opened = open(&src, 10, None).await.ok().expect("open");
    assert!(opened.resync);
    assert_eq!(opened.seq, 3);
    assert_eq!(ids(&opened.snapshot.components), vec!["a", "b"]);
    assert!(!opened.snapshot.truncated);
    assert_eq!(opened.snapshot.epoch, 7);
    assert_eq!(opened.snapshot.service, "toy");
    assert_eq!(opened.snapshot.version, "1.0");
    assert_eq!(src.watch_froms(), vec![Some(3)]);
    assert!(drain(opened).await.is_empty());
}

#[tokio::test]
async fn open_status_with_matching_epoch_and_retained_seq_is_resync_false() {
    let src = Toy::new().with_rows(vec![comp("a")]);
    src.set_epoch(7);
    src.set_window(2, 5);
    for seq in [2, 3, 5] {
        let opened = open(&src, 10, cur(7, seq)).await.ok().expect("open");
        assert!(!opened.resync, "seq {seq} is retained in 2..=5");
        assert_eq!(opened.seq, 5);
        assert_eq!(ids(&opened.snapshot.components), vec!["a"]);
    }
}

#[tokio::test]
async fn open_status_with_other_epoch_or_too_old_seq_is_resync_true_not_an_error() {
    let src = Toy::new().with_rows(vec![comp("a")]);
    src.set_epoch(7);
    src.set_window(2, 5);

    // Other epoch, even with a retained seq.
    let opened = open(&src, 10, cur(8, 3)).await.ok().expect("other epoch");
    assert!(opened.resync);
    assert_eq!(ids(&opened.snapshot.components), vec!["a"]);

    // Seq below the oldest retained.
    let opened = open(&src, 10, cur(7, 1)).await.ok().expect("too old");
    assert!(opened.resync);

    // Seq above the newest emitted.
    let opened = open(&src, 10, cur(7, 6)).await.ok().expect("too new");
    assert!(opened.resync);
    assert_eq!(opened.seq, 5);
}

#[tokio::test]
async fn watch_snapshot_and_changes_have_no_gap_or_duplicate_at_the_boundary() {
    let src = Toy::new().with_rows(vec![comp("x")]);
    src.push(1, comp("c1"));
    src.push(2, comp("c2"));
    src.push(3, comp("c3"));
    // Emitted by the source after the seq was read, while the snapshot is taken.
    src.inject_during_snapshot(4, comp("late"));

    let opened = open(&src, 10, cur(7, 3)).await.ok().expect("open");
    assert_eq!(
        opened.seq, 3,
        "seq is the newest seq read before the snapshot"
    );
    assert!(!opened.resync);
    let changes: Vec<(u64, Component)> = drain(opened)
        .await
        .into_iter()
        .map(|r| r.ok().expect("change"))
        .collect();
    let seqs: Vec<u64> = changes.iter().map(|(s, _)| *s).collect();
    assert_eq!(
        seqs,
        vec![4],
        "backlog <= seq is not replayed, seq 4 appears once"
    );
    assert_eq!(changes[0].1.id, "late");
    assert_eq!(src.watch_froms(), vec![Some(3)]);
}

#[tokio::test]
async fn open_status_overflow_yields_a_coded_transient_lagged_error_and_ends() {
    assert_eq!(WATCH_BUFFER, 256);

    // 300 ready items: some items, then a lagged error, then the end.
    let src = Toy::new();
    fill(&src, 1000, 300);
    let opened = open(&src, 10, None).await.ok().expect("open");
    let mut stream = opened.changes;
    let mut oks = Vec::new();
    let err = loop {
        match stream.next().await {
            Some(Ok((seq, _))) => oks.push(seq),
            Some(Err(e)) => break e,
            None => panic!(
                "stream ended without a lagged error after {} items",
                oks.len()
            ),
        }
    };
    assert!(oks.len() <= WATCH_BUFFER, "got {} items", oks.len());
    let expected: Vec<u64> = (1001..1001 + oks.len() as u64).collect();
    assert_eq!(oks, expected, "items before the error are a gapless prefix");
    assert_eq!(code_of(&err), "op_status.lagged");
    assert_eq!(class_of(&err), ErrorClass::Transient);
    assert!(stream.next().await.is_none(), "stream ends after lagged");

    // Exactly 256 ready items do not lag.
    let src = Toy::new();
    fill(&src, 1000, WATCH_BUFFER as u64);
    let opened = open(&src, 10, None).await.ok().expect("open");
    let all = drain(opened).await;
    assert_eq!(all.len(), WATCH_BUFFER);
    let seqs: Vec<u64> = all.into_iter().map(|r| r.ok().expect("no lag").0).collect();
    assert_eq!(
        seqs,
        (1001..=1000 + WATCH_BUFFER as u64).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn open_status_without_watch_is_a_coded_logical_error() {
    let src = NoWatch::new().with_rows(vec![comp("a")]);
    assert!(!NoWatch::provides_watch());
    let err = open(&src, 10, None).await.err().expect("must fail");
    assert_eq!(code_of(&err), "op_status.watch_unsupported");
    assert_eq!(class_of(&err), ErrorClass::Logical);
}

#[tokio::test]
async fn open_status_snapshot_is_truncated_and_filtered_like_status_get() {
    let mut rows: Vec<Component> = (0..5).map(|i| comp(&format!("r{i}"))).collect();
    rows.push(Component::new("node", "n1", op_status::State::Ok));
    let src = Toy::new().with_rows(rows);

    let room = StatusFilter {
        kind: Some("room".into()),
        id: None,
    };
    let opened = open_status(&src, &room, 3, None).await.ok().expect("open");
    assert_eq!(ids(&opened.snapshot.components), vec!["r0", "r1", "r2"]);
    assert!(opened.snapshot.truncated);

    let opened = open_status(&src, &room, 5, None).await.ok().expect("open");
    assert_eq!(opened.snapshot.components.len(), 5);
    assert!(!opened.snapshot.truncated);

    let one = StatusFilter {
        kind: Some("node".into()),
        id: Some("n1".into()),
    };
    let opened = open_status(&src, &one, 5, None).await.ok().expect("open");
    assert_eq!(ids(&opened.snapshot.components), vec!["n1"]);

    // The same attribute rule as status get.
    let leaky = Toy::new().with_rows(vec![comp("r")
        .attr("count", 2i64)
        .attr("leaked_path", "/home/x")]);
    let result = open(&leaky, 10, None).await;
    if cfg!(debug_assertions) {
        let err = result.err().expect("debug refuses");
        assert_eq!(code_of(&err), "op_status.attr_not_allowed");
        assert!(format!("{err:#}").contains("leaked_path"));
    } else {
        let opened = result.ok().expect("release drops");
        let attrs = &opened.snapshot.components[0].attrs;
        assert_eq!(attrs.len(), 1);
        assert!(attrs.contains_key("count"));
    }
}

#[tokio::test]
async fn open_status_change_items_are_checked_like_the_snapshot() {
    let src = Toy::new();
    fill(&src, 10, 0);
    src.push_tail(Ok((11, comp("ok").attr("count", 1i64))));
    src.push_tail(Ok((
        12,
        comp("bad").attr("count", 1i64).attr("leaked_path", "/x"),
    )));
    let opened = open(&src, 10, None).await.ok().expect("open");
    let items = drain(opened).await;
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].as_ref().ok().expect("first").0, 11);
    if cfg!(debug_assertions) {
        let err = items[1].as_ref().err().expect("debug item error");
        assert_eq!(code_of(err), "op_status.attr_not_allowed");
    } else {
        let (seq, c) = items[1].as_ref().ok().expect("release item");
        assert_eq!(*seq, 12);
        assert!(!c.attrs.contains_key("leaked_path"));
        assert!(c.attrs.contains_key("count"));
    }
}

#[tokio::test]
async fn open_status_works_through_a_dyn_status_source() {
    let src = Toy::new().with_rows(vec![comp("a")]);
    src.push(1, comp("c1"));
    src.set_window(1, 0);
    let dynamic: &dyn StatusSource = &src;
    let opened = open(dynamic, 10, cur(7, 0)).await.ok().expect("open");
    assert!(!opened.resync);
    assert_eq!(opened.seq, 0);
    assert_eq!(ids(&opened.snapshot.components), vec!["a"]);
    let items = drain(opened).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].as_ref().ok().expect("item").0, 1);
}

type Setter = fn(&mut Component, String);

fn setters() -> Vec<(&'static str, Setter)> {
    vec![
        ("kind", |c, v| c.kind = v),
        ("id", |c, v| c.id = v),
        ("last_error_code", |c, v| c.last_error_code = v),
        ("last_error_message", |c, v| c.last_error_message = v),
    ]
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn free_text_fields_over_256_bytes_are_refused_in_debug_and_truncated_in_release() {
    for (field, set) in setters() {
        // Exactly 256 bytes is fine.
        let mut ok = comp("r");
        set(&mut ok, "y".repeat(256));
        let src = Toy::new().with_rows(vec![ok]);
        assert!(open(&src, 10, None).await.is_ok(), "{field} at 256 bytes");

        // 257 bytes in the snapshot.
        let mut bad = comp("r");
        set(&mut bad, too_long());
        let src = Toy::new().with_rows(vec![bad.clone()]);
        let err = open(&src, 10, None).await.err().expect("snapshot refused");
        assert_eq!(code_of(&err), "op_status.field_too_long", "{field}");
        assert_eq!(class_of(&err), ErrorClass::Logical);
        assert!(format!("{err:#}").contains(field), "{field}: {err:#}");

        // 257 bytes in a change item.
        let src = Toy::new();
        fill(&src, 5, 0);
        src.push_tail(Ok((6, bad)));
        let opened = open(&src, 10, None).await.ok().expect("open");
        let items = drain(opened).await;
        let err = items[0].as_ref().err().expect("item refused");
        assert_eq!(code_of(err), "op_status.field_too_long", "{field}");
        assert_eq!(class_of(err), ErrorClass::Logical);
        assert!(format!("{err:#}").contains(field), "{field}: {err:#}");
    }

    for (field, name, version) in [
        ("service", too_long(), "1.0".to_owned()),
        ("version", "toy".to_owned(), too_long()),
    ] {
        let src = Toy::new().with_rows(vec![comp("r")]);
        src.set_service(&name, &version);
        let err = open(&src, 10, None).await.err().expect("service refused");
        assert_eq!(code_of(&err), "op_status.field_too_long", "{field}");
        assert_eq!(class_of(&err), ErrorClass::Logical);
        assert!(format!("{err:#}").contains(field), "{field}: {err:#}");
    }
}

#[cfg(not(debug_assertions))]
#[tokio::test]
async fn free_text_fields_over_256_bytes_are_refused_in_debug_and_truncated_in_release() {
    let long = straddling();
    assert!(long.len() > 256 && !long.is_char_boundary(256));
    for (field, set) in setters() {
        let mut bad = comp("r");
        set(&mut bad, long.clone());
        let pick = |c: &Component| match field {
            "kind" => c.kind.clone(),
            "id" => c.id.clone(),
            "last_error_code" => c.last_error_code.clone(),
            _ => c.last_error_message.clone(),
        };

        let src = Toy::new().with_rows(vec![bad.clone()]);
        let opened = open(&src, 10, None).await.ok().expect("release truncates");
        let got = pick(&opened.snapshot.components[0]);
        assert!(got.len() <= 256, "{field}: {}", got.len());
        assert_eq!(got, straddling_cut(), "{field}");

        let src = Toy::new();
        fill(&src, 5, 0);
        src.push_tail(Ok((6, bad)));
        let opened = open(&src, 10, None).await.ok().expect("open");
        let items = drain(opened).await;
        let got = pick(&items[0].as_ref().ok().expect("item").1);
        assert!(got.len() <= 256, "{field}: {}", got.len());
        assert_eq!(got, straddling_cut(), "{field}");
    }

    let src = Toy::new().with_rows(vec![comp("r")]);
    src.set_service(&long, &long);
    let opened = open(&src, 10, None).await.ok().expect("open");
    assert_eq!(opened.snapshot.service, straddling_cut());
    assert_eq!(opened.snapshot.version, straddling_cut());
}
