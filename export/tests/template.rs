//! Template engine behaviour (T-603b).

use ankhimate_export::template::{Engine, TemplateError, confine};
use serde_json::json;

fn render(body: &str, ctx: serde_json::Value) -> Result<String, TemplateError> {
    Engine::new().render("t", body, &ctx)
}

/// The single most important property here. Non-strict Handlebars renders a
/// missing field as an empty string, so a typo'd `{{nmae}}` yields a bone with
/// no name and an export that looks fine until an engine rejects it.
#[test]
fn a_missing_field_is_an_error_not_an_empty_string() {
    let err = render("name={{nmae}}", json!({"name": "spine"})).unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("nmae"),
        "the error must name the missing field: {text}"
    );
}

/// The editor underlines errors in the template pane, which needs a position.
#[test]
fn a_render_error_carries_its_location() {
    let err = render("line one\nline two {{missing}}", json!({})).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("2"), "the error should report line 2: {text}");
}

#[test]
fn helpers_compose_as_arguments() {
    let out = render(
        "{{round (deg rot) 2}}",
        json!({"rot": std::f64::consts::FRAC_PI_2}),
    )
    .unwrap();
    // "90.0", not "90" — Handlebars prints a float as a float. Both are valid
    // JSON numbers, so this pins the actual behaviour rather than a preference.
    assert_eq!(out.parse::<f64>().unwrap(), 90.0);
}

#[test]
fn degrees_and_radians_round_trip() {
    let out = render("{{round (rad (deg r)) 4}}", json!({"r": 1.2345})).unwrap();
    assert_eq!(out, "1.2345");
}

/// Trailing commas are invalid JSON, and a format that emits one is rejected by
/// every parser it reaches.
#[test]
fn each_with_last_places_separators_correctly() {
    let out = render(
        "[{{#each xs}}{{this}}{{#unless @last}},{{/unless}}{{/each}}]",
        json!({"xs": [1, 2, 3]}),
    )
    .unwrap();
    assert_eq!(out, "[1,2,3]");
    assert!(serde_json::from_str::<serde_json::Value>(&out).is_ok());
}

#[test]
fn a_loop_can_reach_the_enclosing_scope() {
    let out = render(
        "{{#each xs}}{{../unit}}:{{this}} {{/each}}",
        json!({"unit": "deg", "xs": [1, 2]}),
    )
    .unwrap();
    assert_eq!(out, "deg:1 deg:2 ");
}

/// Templates emit JSON, Lua and XML — never HTML. HTML-escaping would turn
/// every `&` and quote in a name into an entity and corrupt the output.
#[test]
fn output_is_not_html_escaped() {
    let out = render("{{name}}", json!({"name": "arm & leg <L>"})).unwrap();
    assert_eq!(out, "arm & leg <L>");
}

#[test]
fn the_round_helper_does_not_emit_negative_zero() {
    // -0 is valid JSON but a gratuitous diff against the 0 another rig writes
    // for the same rest pose. The digits may be "0" or "0.0"; the sign is what
    // this pins.
    let out = render("{{round v 3}}", json!({"v": -0.0001})).unwrap();
    assert!(!out.starts_with('-'), "rounded to negative zero: {out}");
    assert_eq!(out.parse::<f64>().unwrap(), 0.0);
}

#[test]
fn hex_converts_rgba_floats_to_eight_digits() {
    let out = render("{{hex c}}", json!({"c": [1.0, 0.0, 0.5, 1.0]})).unwrap();
    assert_eq!(out, "ff0080ff");
}

#[test]
fn pad_zero_fills_for_frame_numbers() {
    let out = render("frame_{{pad n 4}}.png", json!({"n": 7})).unwrap();
    assert_eq!(out, "frame_0007.png");
}

#[test]
fn or_bridges_optional_channels() {
    let ctx = json!({"a": [], "b": [1]});
    assert_eq!(
        render("{{#if (or a b)}}y{{else}}n{{/if}}", ctx).unwrap(),
        "y"
    );
    let empty = json!({"a": [], "b": []});
    assert_eq!(
        render("{{#if (or a b)}}y{{else}}n{{/if}}", empty).unwrap(),
        "n"
    );
}

/// `{{#if}}` on an absent key must *not* trip strict mode, or optional channels
/// become inexpressible. `{{#each}}` on an absent key does error — which is why
/// the context always emits its collections, empty rather than missing.
#[test]
fn if_tolerates_an_absent_key_even_in_strict_mode() {
    let out = render("{{#if nope}}y{{else}}n{{/if}}", json!({})).unwrap();
    assert_eq!(out, "n");
}

#[test]
fn rendering_is_deterministic() {
    let ctx = json!({"xs": [{"n": "a"}, {"n": "b"}, {"n": "c"}]});
    let body = "{{#each xs}}{{n}}{{/each}}";
    let first = render(body, ctx.clone()).unwrap();
    let second = render(body, ctx).unwrap();
    assert_eq!(first, second);
}

// ── Path confinement ────────────────────────────────────────────────────
//
// `output_path` is itself a template, so it renders from user data: a preset
// from another studio, a rig from another artist. Both can carry a traversal.

#[test]
fn a_normal_path_is_kept() {
    assert_eq!(confine("t", "anim/walk.json").unwrap(), "anim/walk.json");
}

#[test]
fn redundant_segments_are_normalised_away() {
    assert_eq!(confine("t", "./anim//walk.json").unwrap(), "anim/walk.json");
    assert_eq!(confine("t", "a/b/../walk.json").unwrap(), "a/walk.json");
}

#[test]
fn a_path_climbing_above_the_output_directory_is_rejected() {
    for attempt in [
        "../escape.txt",
        "a/../../escape.txt",
        "../../../../etc/passwd",
    ] {
        match confine("t", attempt) {
            Err(TemplateError::EscapingPath { .. }) => {}
            other => panic!("'{attempt}' should have been rejected, got {other:?}"),
        }
    }
}

#[test]
fn an_absolute_path_is_rejected() {
    for attempt in ["/etc/passwd", "C:/Windows/system32/x.dll", "D:\\project\\x"] {
        match confine("t", attempt) {
            Err(TemplateError::EscapingPath { .. }) => {}
            other => panic!("'{attempt}' should have been rejected, got {other:?}"),
        }
    }
}

/// Backslashes are separators on Windows, so a traversal written with them has
/// to be caught too.
#[test]
fn a_windows_style_traversal_is_rejected() {
    match confine("t", "..\\..\\escape.txt") {
        Err(TemplateError::EscapingPath { .. }) => {}
        other => panic!("expected rejection, got {other:?}"),
    }
}

/// The realistic vector: not a hand-written path, but a bone or animation named
/// `..` flowing into `anim/{{animation.name}}.json`.
#[test]
fn a_traversal_arriving_through_a_name_is_rejected() {
    let rendered = render("anim/{{name}}.json", json!({"name": "../../../../evil"})).unwrap();
    match confine("t", &rendered) {
        Err(TemplateError::EscapingPath { .. }) => {}
        other => panic!("expected rejection of {rendered:?}, got {other:?}"),
    }
}

#[test]
fn an_empty_path_is_rejected() {
    match confine("t", "  /  ") {
        Err(TemplateError::EmptyPath { .. }) | Err(TemplateError::EscapingPath { .. }) => {}
        other => panic!("expected rejection, got {other:?}"),
    }
    match confine("t", "") {
        Err(TemplateError::EmptyPath { .. }) => {}
        other => panic!("expected EmptyPath, got {other:?}"),
    }
}
