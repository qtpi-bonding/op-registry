mod common;

use common::follow::*;
use common::*;
use op_listen::{follow, FollowEnd, FollowOptions};

#[tokio::test(start_paused = true)]
async fn follow_json_lines_are_byte_identical() {
    let (connect, _connects) = scripted(vec![
        Step::Frames(vec![
            Ok(snap(false, "0:1")),
            Ok(item("a", "0:2")),
            Err(transient()),
        ]),
        frames(vec![snap(true, "0:9"), stop("op_listen.revoked")]),
    ]);
    let (sink, lines) = make_sink();
    let end = follow(connect, sink, FollowOptions::default())
        .await
        .expect("follow ends cleanly");
    assert!(matches!(end, FollowEnd::Stopped { ref code } if code == "op_listen.revoked"));
    let got = lines.lock().unwrap().clone();
    let want = [
        r#"{"frame":"snapshot","snapshot":{"k":"v"},"resync":false,"cursor":"0:1"}"#,
        r#"{"frame":"item","item":"a","cursor":"0:2"}"#,
        r#"{"marker":"reconnected","cursor":"0:2","resync":true}"#,
        r#"{"frame":"snapshot","snapshot":{"k":"v"},"resync":true,"cursor":"0:9"}"#,
        r#"{"frame":"stop","code":"op_listen.revoked","message":"gone"}"#,
        r#"{"marker":"stopped","code":"op_listen.revoked"}"#,
    ];
    assert_eq!(got, want);
}
