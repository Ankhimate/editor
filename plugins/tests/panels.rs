//! Plugin panels, declared in JavaScript and read back as widgets.
//!
//! The design under test: a plugin returns a **list of widgets** and the host
//! paints them. A script is never called from inside a paint loop and never
//! holds a `&mut Ui`, which is what makes an immediate-mode UI safe to extend
//! at all.

use ankhimate_plugins::Host;
use ankhimate_plugins::panel::{PanelAction, PickKind, Widget};

const PANEL: &str = r#"
ankhimate.registerPanel({
  id: "tools.mirror",
  title: "Mirror",
  build() {
    const bones = names().bones;
    return [
      { heading: "Mirror" },
      { text: `${bones.length} bones`, weak: true },
      { separator: true },
      { pick: "Source", on: "source", of: "bone", value: bones[0] ?? "" },
      { checkbox: "Include children", on: "children", value: true },
      { number: "Offset", on: "offset", value: 10, min: -100, max: 100 },
      { button: "Mirror", on: "go", disabled: bones.length === 0 },
    ];
  },
  on(action, value) {
    if (action === "go") {
      ops.invoke("bone.create", { name: "mirrored" });
    }
    if (action === "offset") {
      ops.invoke("bone.create", { name: `offset_${value}` });
    }
  },
});
"#;

fn edit_with_a_bone() -> ankhimate_document::Edit {
    let host = Host::new();
    let mut edit = ankhimate_document::Edit::default();
    host.run(r#"ops.invoke("bone.create", { name: "root" });"#, &mut edit)
        .expect("the setup runs");
    edit
}

#[test]
fn a_plugin_declares_a_panel_without_building_it() {
    // What a host does at load time: run the file, collect what it declared,
    // put those in the View menu. Building here would be describing a document
    // nobody has opened.
    let host = Host::new();
    let panels = host.panels(PANEL).expect("the plugin loads");

    assert_eq!(panels.len(), 1);
    assert_eq!(panels[0].id, "tools.mirror");
    assert_eq!(panels[0].title, "Mirror");
}

#[test]
fn a_panel_returns_widgets_the_host_can_draw() {
    let host = Host::new();
    let mut edit = edit_with_a_bone();
    let widgets = host
        .build_panel(PANEL, "tools.mirror", &mut edit)
        .expect("the panel builds");

    assert!(matches!(widgets[0], Widget::Heading { .. }));
    assert!(matches!(widgets[2], Widget::Separator { .. }));

    let Widget::Pick { of, value, .. } = &widgets[3] else {
        panic!("expected a picker, got {:?}", widgets[3]);
    };
    assert_eq!(*of, PickKind::Bone);
    assert_eq!(value, "root", "the panel read the rig it was given");
}

#[test]
fn a_panel_sees_the_document_it_is_built_against() {
    // The whole point of rebuilding on change rather than caching one answer.
    let host = Host::new();

    let mut empty = ankhimate_document::Edit::default();
    let widgets = host
        .build_panel(PANEL, "tools.mirror", &mut empty)
        .expect("builds against an empty rig");
    let Widget::Button { disabled, .. } = widgets.last().expect("the button") else {
        panic!("expected a button");
    };
    assert!(*disabled, "nothing to mirror, so the button is off");

    let mut populated = edit_with_a_bone();
    let widgets = host
        .build_panel(PANEL, "tools.mirror", &mut populated)
        .expect("builds against a rig with bones");
    let Widget::Button { disabled, .. } = widgets.last().expect("the button") else {
        panic!("expected a button");
    };
    assert!(!*disabled, "and on once there is");
}

#[test]
fn touching_a_widget_reaches_the_plugin_and_the_document() {
    // The half that makes a panel more than a display: the handler runs against
    // the real document, so its edits land where an undo can reach them.
    let host = Host::new();
    let mut edit = edit_with_a_bone();

    host.panel_action(
        PANEL,
        "tools.mirror",
        &PanelAction {
            action: "go".into(),
            value: serde_json::Value::Null,
        },
        &mut edit,
    )
    .expect("the action runs");

    assert!(
        edit.doc
            .skeleton
            .bones
            .iter()
            .any(|(_, b)| b.name == "mirrored"),
        "the plugin's handler invoked a verb"
    );
}

#[test]
fn a_widgets_value_reaches_the_handler() {
    // A button carries nothing; everything else carries what the user set. One
    // shape for both, or the host would need a second enum to keep in step with
    // the widget one.
    let host = Host::new();
    let mut edit = edit_with_a_bone();

    host.panel_action(
        PANEL,
        "tools.mirror",
        &PanelAction {
            action: "offset".into(),
            value: serde_json::json!(42),
        },
        &mut edit,
    )
    .expect("the action runs");

    assert!(
        edit.doc
            .skeleton
            .bones
            .iter()
            .any(|(_, b)| b.name == "offset_42"),
        "the value arrived, not just the action name"
    );
}

#[test]
fn an_unknown_panel_id_is_an_error_with_the_id_in_it() {
    let host = Host::new();
    let mut edit = ankhimate_document::Edit::default();
    let error = host
        .build_panel(PANEL, "tools.nope", &mut edit)
        .expect_err("no such panel");

    assert!(
        error.to_string().contains("tools.nope"),
        "the reason names what was asked for: {error}"
    );
}

#[test]
fn a_widget_this_build_does_not_know_is_an_error_rather_than_a_gap() {
    // A panel silently missing the control its author wrote reads as the host
    // being broken, and they have no way in. Failing names the problem.
    let host = Host::new();
    let mut edit = ankhimate_document::Edit::default();
    let error = host
        .build_panel(
            r#"
            ankhimate.registerPanel({
              id: "p", title: "P",
              build: () => [{ slider: "Amount", on: "a" }],
            });
            "#,
            "p",
            &mut edit,
        )
        .expect_err("no such widget");

    assert!(
        !error.to_string().is_empty(),
        "and the failure says something"
    );
}

#[test]
fn a_panel_that_throws_says_why() {
    // The lesson from the importer host: a thrown message dies with the scope
    // it was thrown in, and every failure comes back as "Exception generated by
    // QuickJS" unless it is captured inside.
    let host = Host::new();
    let mut edit = ankhimate_document::Edit::default();
    let error = host
        .build_panel(
            r#"
            ankhimate.registerPanel({
              id: "p", title: "P",
              build() { throw new Error("the rig has no head bone"); },
            });
            "#,
            "p",
            &mut edit,
        )
        .expect_err("the panel threw");

    assert!(
        error.to_string().contains("no head bone"),
        "the author's own message survived: {error}"
    );
}
