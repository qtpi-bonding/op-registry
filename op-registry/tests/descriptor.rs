use op_registry::{
    derive_name, Access, Descriptor, FieldKind, FieldMeta, OpGlobals, OpParams, Patch, SubjectMeta,
};
use std::path::PathBuf;

#[test]
fn name_joins_words_and_snakes_hyphens() {
    let path = ["dispatch-target".to_owned(), "list".to_owned()];
    assert_eq!(derive_name(&path), "dispatch_target_list");
}

#[test]
fn one_word_path_is_bare_verb() {
    let mut d = Descriptor::default();
    d.path = vec!["index".to_owned()];
    d.name = derive_name(&d.path);
    assert_eq!(d.name, "index");
    assert_eq!(d.verb(), "index");
    assert!(d.noun_path().is_empty());
}

#[test]
fn noun_path_is_all_words_but_the_last() {
    let mut d = Descriptor::default();
    d.path = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
    assert_eq!(d.verb(), "c");
    assert_eq!(d.noun_path(), ["a".to_owned(), "b".to_owned()]);
}

#[test]
fn access_from_words_destructive_writes() {
    let a = Access::from_words(&["destructive"]);
    assert!(a.destructive && a.writes && !a.reads);
    let b = Access::from_words(&["reads", "network", "spawns"]);
    assert!(b.reads && b.network && b.spawns && !b.writes && !b.destructive);
    assert_eq!(Access::from_words(&[]), Access::default());
}

#[test]
fn field_meta_new_derives_flag_and_key() {
    let f = FieldMeta::new("owner_key", FieldKind::Scalar);
    assert_eq!(f.rust_name, "owner_key");
    assert_eq!(f.flag, "owner-key");
    assert_eq!(f.key, "ownerKey");
    assert_eq!(f.help, "");
    assert!(!f.required && !f.secret);
    assert!(f.from_group.is_none() && f.subject.is_none());
    assert!(f.env.is_none() && f.prompt.is_none());
    let f = f
        .with_flag("k")
        .with_key("kk")
        .with_help("h")
        .required(true)
        .secret(true)
        .with_env("E")
        .with_prompt("P")
        .with_subject(SubjectMeta::new("agent").id_only(true));
    assert_eq!(
        (f.flag.as_str(), f.key.as_str(), f.help.as_str()),
        ("k", "kk", "h")
    );
    assert!(f.required && f.secret);
    assert_eq!(f.env.as_deref(), Some("E"));
    assert_eq!(f.prompt.as_deref(), Some("P"));
    let s = f.subject.unwrap();
    assert_eq!(
        (s.kind.as_str(), s.id_only, s.absent_ok),
        ("agent", true, false)
    );
    assert!(SubjectMeta::new("a").absent_ok(true).absent_ok);
}

#[allow(dead_code)]
#[derive(Debug, OpParams)]
struct Kinds {
    dir: PathBuf,
    maybe_dir: Option<PathBuf>,
    #[op(clearable)]
    name: Patch<String>,
}

fn kind_of(fields: &[FieldMeta], name: &str) -> FieldKind {
    fields.iter().find(|f| f.rust_name == name).unwrap().kind
}

#[test]
fn pathbuf_field_reports_path_kind() {
    let fields = Kinds::fields();
    assert_eq!(kind_of(&fields, "dir"), FieldKind::Path);
    assert_eq!(kind_of(&fields, "maybe_dir"), FieldKind::Path);
}

#[test]
fn patch_field_reports_patch_kind() {
    assert_eq!(kind_of(&Kinds::fields(), "name"), FieldKind::Patch);
}

#[allow(dead_code)]
#[derive(Debug, OpGlobals)]
struct EnvG {
    #[op(env = "DESC_TEST_HUB_UNIQUE")]
    hub: Option<String>,
    plain: bool,
}

#[test]
fn global_field_meta_has_env() {
    let fields = EnvG::fields();
    let hub = fields.iter().find(|f| f.rust_name == "hub").unwrap();
    assert_eq!(hub.env.as_deref(), Some("DESC_TEST_HUB_UNIQUE"));
    let plain = fields.iter().find(|f| f.rust_name == "plain").unwrap();
    assert_eq!(plain.env, None);
}
