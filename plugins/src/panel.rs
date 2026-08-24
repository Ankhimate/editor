//! Panels a plugin declares and the host draws.
//!
//! **A widget vocabulary, not a plugin-draws-egui API.** The plugin returns a
//! list of widgets; the host paints them. That dissolves the immediate-mode
//! problem — a script cannot be called from inside a paint loop and cannot hold
//! a `&mut Ui` — and it matches what Blender does: Python addons call
//! `layout.prop()`, they do not draw pixels.
//!
//! The cost, stated plainly: a plugin can only use a widget the host ships. A
//! thumbnail strip exists here because it was decided to ship one; anything not
//! in [`Widget`] cannot be drawn at all. That is why Blender addon UIs look
//! alike, and it is the right trade — the alternative is every plugin owning
//! its own paint code and its own bugs in ours.
//!
//! # When `build` runs
//!
//! **Not every frame.** A panel is rebuilt when the document changes or when
//! one of its own widgets is touched; otherwise the host redraws the widget
//! list it already has. Running a JS context sixty times a second per panel is
//! exactly the cost this design exists to avoid, and panel contents change when
//! the document does, not when the compositor asks.
//!
//! The consequence a plugin author must know: `build` is **not** a place to
//! read a clock or animate anything. It describes what the panel shows for a
//! given document, and the host decides when to ask.
//!
//! # A public contract
//!
//! These field names are a rename away from breaking every plugin that uses
//! them, with no compiler on that side — the same rule `docs/export-context.md`
//! states for the template context.

use serde::{Deserialize, Serialize};

/// Transient editor-view changes requested by a panel action.
///
/// These are deliberately not document verbs: hiding artwork for a preview is
/// session state, so it must not be saved, keyed, or placed on the undo stack.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PanelEffect {
    /// Slot name to visible state. Names keep the plugin boundary stable.
    #[serde(default)]
    pub slot_visibility: std::collections::BTreeMap<String, bool>,
}

/// A panel a plugin contributes, as declared on load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PanelSpec {
    /// Dotted id, unique across plugins — `tools.mirror`.
    ///
    /// The same naming rule verbs follow, and for the same reason: a keymap, a
    /// View menu entry and a saved layout all reach a panel through this.
    pub id: String,
    /// What the tab says.
    pub title: String,
}

/// One row of a panel.
///
/// Untagged so a plugin writes `{ button: "Go", on: "go" }` rather than
/// `{ kind: "button", ... }`: the shape is the tag, which is how a declarative
/// list stays readable in the language it is written in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Widget {
    /// A heading, above a group of related rows.
    Heading { heading: String },
    /// Static text.
    Text {
        text: String,
        /// Draw it dimmed — for the explanation under a control.
        #[serde(default)]
        weak: bool,
    },
    /// A horizontal rule.
    Separator { separator: bool },
    /// A button. `on` is the action name handed back to the plugin.
    Button {
        button: String,
        on: String,
        /// Grey it out and ignore clicks. A reason belongs in `tooltip`.
        #[serde(default)]
        disabled: bool,
        #[serde(default)]
        tooltip: Option<String>,
    },
    /// A host-mediated file picker. The action value is an array of
    /// `{ name, bytes_base64 }` objects and is never retained as panel state.
    File {
        file: String,
        on: String,
        #[serde(default)]
        extensions: Vec<String>,
        #[serde(default)]
        multiple: bool,
        #[serde(default)]
        tooltip: Option<String>,
    },
    /// A tick box.
    Checkbox {
        checkbox: String,
        on: String,
        #[serde(default)]
        value: bool,
        #[serde(default)]
        tooltip: Option<String>,
    },
    /// A number, dragged or typed.
    Number {
        number: String,
        on: String,
        #[serde(default)]
        value: f64,
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
        /// How fast a drag moves it. Defaults to something sane for the range.
        #[serde(default)]
        speed: Option<f64>,
        #[serde(default)]
        tooltip: Option<String>,
    },
    /// A line of text.
    Text_ {
        field: String,
        on: String,
        #[serde(default)]
        value: String,
        #[serde(default)]
        tooltip: Option<String>,
    },
    /// A dropdown over choices the plugin supplies.
    Choice {
        choice: String,
        on: String,
        options: Vec<String>,
        #[serde(default)]
        value: String,
        #[serde(default)]
        tooltip: Option<String>,
    },
    /// A dropdown over something the *rig* has — bones, slots, animations,
    /// skins, images.
    ///
    /// The host fills the list from the open document. A plugin that had to
    /// build it would be showing a list that goes stale the moment a bone is
    /// renamed, and every plugin would keep it fresh differently.
    Pick {
        pick: String,
        on: String,
        of: PickKind,
        #[serde(default)]
        value: String,
        #[serde(default)]
        tooltip: Option<String>,
    },
    /// A scrollable list of rows, one selectable.
    ///
    /// For "pick one of these" beyond the rig's own vocabulary — a plugin's own
    /// presets, a list of files it found.
    List {
        list: Vec<String>,
        on: String,
        /// Which row is selected, or `None`.
        #[serde(default)]
        selected: Option<usize>,
        /// Rows before it scrolls.
        #[serde(default)]
        rows: Option<usize>,
    },
    /// A strip of image thumbnails from the asset library, one selectable.
    ///
    /// Named in `docs/plugin-plan.md` as the example of a widget that only
    /// exists because the host ships it. It does, because picking art by
    /// looking at it is the one thing a name-only list cannot do.
    Thumbnails {
        thumbnails: Vec<String>,
        on: String,
        #[serde(default)]
        selected: Option<String>,
        /// Edge length in points. The host clamps it to something drawable.
        #[serde(default)]
        size: Option<f32>,
    },
}

/// What a [`Widget::Pick`] lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PickKind {
    Bone,
    Slot,
    Animation,
    Skin,
    Image,
}

/// A widget the user touched, on its way back to the plugin.
///
/// One shape for every widget: the action name the plugin gave, and whatever the
/// widget produced. A button produces nothing, a checkbox a bool, a picker a
/// name — so the value is JSON rather than a second enum to keep in step with
/// the first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PanelAction {
    /// The `on` field of the widget that was touched.
    pub action: String,
    /// What it produced. `null` for a button.
    pub value: serde_json::Value,
    /// Every widget's current value, by action name.
    ///
    /// **The host tracks these, not the plugin.** A fresh runtime is built per
    /// call so a plugin cannot hold state across an undo — which also means
    /// `this.name = value` in one handler is gone by the next, and a panel that
    /// tried it would read every field as `undefined`. That was not a
    /// hypothetical: the first plugin written against this API did exactly that
    /// and created nine bones all called `new_bone`.
    ///
    /// So the panel's state lives where it can survive: here. A handler reading
    /// `state.name` gets what the user typed, whether they typed it this call or
    /// four calls ago.
    #[serde(default)]
    pub state: serde_json::Map<String, serde_json::Value>,
}

impl Widget {
    /// The action name this widget reports under, if it reports at all.
    pub fn action(&self) -> Option<&str> {
        match self {
            Widget::Heading { .. } | Widget::Text { .. } | Widget::Separator { .. } => None,
            Widget::Button { on, .. }
            | Widget::File { on, .. }
            | Widget::Checkbox { on, .. }
            | Widget::Number { on, .. }
            | Widget::Text_ { on, .. }
            | Widget::Choice { on, .. }
            | Widget::Pick { on, .. }
            | Widget::List { on, .. }
            | Widget::Thumbnails { on, .. } => Some(on),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Widget {
        serde_json::from_str(json).expect("a widget")
    }

    #[test]
    fn the_shape_is_the_tag() {
        // `{ button: "Go", on: "go" }` rather than `{ kind: "button", … }`.
        // A declarative list has to stay readable in the language it is written
        // in, and JavaScript is not a language where a discriminant field reads
        // naturally.
        assert!(matches!(
            parse(r#"{"button":"Go","on":"go"}"#),
            Widget::Button { .. }
        ));
        assert!(matches!(
            parse(r#"{"file":"Import","on":"import","extensions":["pack"]}"#),
            Widget::File { .. }
        ));
        assert!(matches!(parse(r#"{"text":"hello"}"#), Widget::Text { .. }));
        assert!(matches!(
            parse(r#"{"heading":"Options"}"#),
            Widget::Heading { .. }
        ));
    }

    #[test]
    fn every_interactive_widget_names_its_action() {
        // A widget the user can touch and that reports nothing is a control
        // whose clicks vanish — which reads to the plugin author as the host
        // being broken.
        let interactive = [
            parse(r#"{"button":"Go","on":"go"}"#),
            parse(r#"{"file":"Import","on":"import","extensions":["pack"]}"#),
            parse(r#"{"checkbox":"On","on":"on"}"#),
            parse(r#"{"number":"Count","on":"count"}"#),
            parse(r#"{"field":"Name","on":"name"}"#),
            parse(r#"{"choice":"Mode","on":"mode","options":["a"]}"#),
            parse(r#"{"pick":"Bone","on":"bone","of":"bone"}"#),
            parse(r#"{"list":["a"],"on":"row"}"#),
            parse(r#"{"thumbnails":["img"],"on":"art"}"#),
        ];
        for widget in &interactive {
            assert!(widget.action().is_some(), "no action name on {widget:?}");
        }

        for inert in [
            parse(r#"{"text":"hello"}"#),
            parse(r#"{"heading":"Options"}"#),
            parse(r#"{"separator":true}"#),
        ] {
            assert_eq!(inert.action(), None);
        }
    }

    #[test]
    fn a_picker_names_what_it_lists() {
        let Widget::Pick { of, .. } = parse(r#"{"pick":"Bone","on":"b","of":"animation"}"#) else {
            panic!("not a picker");
        };
        assert_eq!(of, PickKind::Animation);
    }

    #[test]
    fn optional_fields_may_be_left_out() {
        // A plugin writing `{ button: "Go", on: "go" }` should not have to
        // spell out four defaults it does not care about.
        let Widget::Button {
            disabled, tooltip, ..
        } = parse(r#"{"button":"Go","on":"go"}"#)
        else {
            panic!("not a button");
        };
        assert!(!disabled);
        assert_eq!(tooltip, None);
    }

    #[test]
    fn an_unknown_widget_shape_is_an_error_rather_than_a_guess() {
        // Untagged enums fall through to the last matching variant, which is
        // how a typo becomes a silently different widget. Nothing matches here,
        // so it must fail rather than draw something the author did not write.
        let result: Result<Widget, _> = serde_json::from_str(r#"{"slider":"Amount","on":"a"}"#);
        assert!(result.is_err(), "got {result:?}");
    }
}
