use vk::{locale_is_utf8, parse_issue_reference, parse_pr_reference};

#[test]
fn parse_url() {
    let (repo, number) =
        parse_pr_reference("https://github.com/owner/repo/pull/42", None).expect("valid reference");
    assert_eq!(repo.owner, "owner");
    assert_eq!(repo.name, "repo");
    assert_eq!(number, 42);
}

#[test]
fn parse_url_git_suffix() {
    let (repo, number) = parse_pr_reference("https://github.com/owner/repo.git/pull/7", None)
        .expect("valid reference");
    assert_eq!(repo.owner, "owner");
    assert_eq!(repo.name, "repo");
    assert_eq!(number, 7);
}

#[test]
fn parse_url_plural_segment() {
    let (repo, number) = parse_pr_reference("https://github.com/owner/repo/pulls/13", None)
        .expect("valid reference");
    assert_eq!(repo.owner, "owner");
    assert_eq!(repo.name, "repo");
    assert_eq!(number, 13);
}

#[test]
fn parse_pr_number_with_repo() {
    let (repo, number) = parse_pr_reference("5", Some("foo/bar")).expect("valid ref");
    assert_eq!(repo.owner, "foo");
    assert_eq!(repo.name, "bar");
    assert_eq!(number, 5);
}

#[test]
fn parse_issue_number_with_repo() {
    let (repo, number) = parse_issue_reference("8", Some("baz/qux")).expect("valid ref");
    assert_eq!(repo.owner, "baz");
    assert_eq!(repo.name, "qux");
    assert_eq!(number, 8);
}

#[test]
fn locale_detection() {
    unsafe { std::env::set_var("LC_ALL", "en_GB.UTF-8") };
    assert!(locale_is_utf8());
    unsafe { std::env::set_var("LC_ALL", "en_GB.UTF80") };
    assert!(!locale_is_utf8());
    unsafe { std::env::remove_var("LC_ALL") };
    unsafe { std::env::set_var("LC_CTYPE", "en_GB.UTF-8") };
    assert!(locale_is_utf8());
    unsafe { std::env::set_var("LC_CTYPE", "C") };
    assert!(!locale_is_utf8());
}
