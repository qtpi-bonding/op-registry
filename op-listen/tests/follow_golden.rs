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
        r#"{"cursor":"0:1","frame":"snapshot","resync":false,"snapshot":{"k":"v"}}"#,
        r#"{"cursor":"0:2","frame":"item","item":"a"}"#,
        r#"{"cursor":"0:2","marker":"reconnected","resync":true}"#,
        r#"{"cursor":"0:9","frame":"snapshot","resync":true,"snapshot":{"k":"v"}}"#,
        r#"{"code":"op_listen.revoked","frame":"stop","message":"gone"}"#,
        r#"{"code":"op_listen.revoked","marker":"stopped"}"#,
    ];
    assert_eq!(got, want);
}
