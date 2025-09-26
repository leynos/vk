//! Behavioural tests verifying CLI argument serialisation for pull request flags.
//!
//! Confirms boolean fields omit default values so configuration sources can override CLI defaults.

use rstest::rstest;
use serde_json::json;
use vk::PrArgs;

#[rstest]
#[case(false)]
#[case(true)]
fn serialises_show_outdated_field(#[case] flag: bool) {
    let args = PrArgs {
        reference: Some(String::from("ref")),
        show_outdated: flag,
        ..PrArgs::default()
    };

    let value = serde_json::to_value(&args).expect("serialise pr args");
    if flag {
        assert_eq!(value.get("show_outdated"), Some(&json!(true)));
    } else {
        assert!(value.get("show_outdated").is_none());
    }
}
