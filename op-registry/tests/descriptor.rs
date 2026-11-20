use op_registry::{derive_name, Access, Descriptor};

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
