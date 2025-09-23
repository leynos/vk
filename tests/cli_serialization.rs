//! Behavioural tests verifying CLI argument serialisation for pull request flags.
//!
//! Confirms boolean fields omit default values so configuration sources can override CLI defaults.

use serde_json::json;
use vk::PrArgs;

#[test]
fn omits_show_outdated_when_false() {
    let args = PrArgs {
        reference: Some(String::from("ref")),
        show_outdated: false,
        ..PrArgs::default()
    };

    let value = serde_json::to_value(&args).expect("serialise pr args");
    assert!(value.get("show_outdated").is_none());
}

#[test]
fn includes_show_outdated_when_true() {
    let args = PrArgs {
        reference: Some(String::from("ref")),
        show_outdated: true,
        ..PrArgs::default()
    };

    let value = serde_json::to_value(&args).expect("serialise pr args");
    assert_eq!(value.get("show_outdated"), Some(&json!(true)));
}
