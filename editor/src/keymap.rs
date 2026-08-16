//! Key bindings as data (T-701).
//!
//! Every shortcut used to be an `if ctx.input(|i| i.key_pressed(..))` arm in
//! `App::update`, which meant the bindings could not be listed, could not be
//! rebound, and could not be discovered by a plugin. This is the same table,
//! addressable.
//!
//! A binding names an [`Operator`](crate::commands::registry::Operator) by id
//! rather than pointing at a function, so a plugin that shadows `edit.undo`
//! inherits every key bound to it without touching the keymap, and a keymap
//! written by a user survives a plugin being installed or removed.
//!
//! # Unknown ids do not fail the load
//!
//! A binding for an operator that no longer exists — a plugin uninstalled, a
//! built-in renamed — is kept in the table and simply resolves to nothing when
//! pressed. Dropping it would silently rewrite the user's file the first time
//! they ran without a plugin; refusing the file would cost them every *other*
//! binding over one stale line. This mirrors how `Config::torn_off` treats a
//! pane name it does not recognise.
//!
//! # Modifiers are matched exactly
//!
//! A binding for `S` does **not** fire on `Ctrl+S`. The old inline handlers had
//! to guard for this by hand — the gizmo block carried `!ctrl && !shift` with a
//! comment explaining that `Shift+H` was isolation and would otherwise trip
//! Shear — and every new binding was one forgotten guard away from a collision.
//! Exact matching makes that structural: `Shift+H` and `H` are different keys,
//! and neither can shadow the other by accident.

use eframe::egui::{self, Key, Modifiers};
use serde::{Deserialize, Serialize};

/// A chord: one key plus the exact modifier set it fires under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Chord {
    /// Serialized by egui's own name (`"A"`, `"F2"`, `"Num1"`), so the file is
    /// readable and a hand-edited keymap is plausible.
    pub key: Key,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
}

impl Chord {
    pub const fn plain(key: Key) -> Self {
        Self {
            key,
            ctrl: false,
            shift: false,
            alt: false,
        }
    }

    pub const fn ctrl(key: Key) -> Self {
        Self {
            ctrl: true,
            ..Self::plain(key)
        }
    }

    pub const fn ctrl_shift(key: Key) -> Self {
        Self {
            ctrl: true,
            shift: true,
            ..Self::plain(key)
        }
    }

    /// Does `mods` match this chord exactly?
    ///
    /// Exactly, not "at least": see the module note on why `S` must not fire
    /// under `Ctrl+S`. `Modifiers::command` is deliberately not consulted —
    /// egui maps it to Cmd on macOS and Ctrl elsewhere, and a keymap file that
    /// means different keys on different platforms is worse than one that is
    /// explicit.
    pub fn matches(&self, mods: &Modifiers) -> bool {
        mods.ctrl == self.ctrl && mods.shift == self.shift && mods.alt == self.alt
    }

    /// Human-readable, for menus and the keymap editor: `"Ctrl+Shift+S"`.
    pub fn label(&self) -> String {
        let mut out = String::new();
        for (on, name) in [
            (self.ctrl, "Ctrl"),
            (self.shift, "Shift"),
            (self.alt, "Alt"),
        ] {
            if on {
                out.push_str(name);
                out.push('+');
            }
        }
        out.push_str(self.key.name());
        out
    }
}

/// One row of the table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Binding {
    pub chord: Chord,
    /// The operator id this fires. Not validated on load — see the module note.
    pub operator: String,
    /// Fire even while a text field has focus.
    ///
    /// Off for nearly everything: typing a bone name must not scrub the
    /// timeline or swap the tool. `Ctrl+Z` is the exception that matters —
    /// undo inside a text field is undo, and refusing it there would be a
    /// worse surprise than the one this flag exists to prevent.
    #[serde(default)]
    pub while_typing: bool,
}

impl Binding {
    fn new(chord: Chord, operator: &str) -> Self {
        Self {
            chord,
            operator: operator.to_string(),
            while_typing: false,
        }
    }

    fn while_typing(chord: Chord, operator: &str) -> Self {
        Self {
            while_typing: true,
            ..Self::new(chord, operator)
        }
    }
}

/// The full set of bindings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::builtin()
    }
}

impl Keymap {
    /// The shipped bindings — the ones that were hardcoded in `App::update`.
    ///
    /// Order matters only for [`chord_for`](Self::chord_for), which reports the
    /// *first* chord bound to an operator as the one a menu should display.
    pub fn builtin() -> Self {
        use Key::*;
        let bindings = vec![
            // ── Edit ────────────────────────────────────────────────────────
            // Undo and redo work inside a text field; the clipboard keys below
            // deliberately do not, since egui's own text editing owns them
            // there. That split is exactly what the old inline handler encoded
            // by putting these three above its `typing_now` guard and the rest
            // below it.
            Binding::while_typing(Chord::ctrl(Z), "edit.undo"),
            Binding::while_typing(Chord::ctrl_shift(Z), "edit.redo"),
            Binding::while_typing(Chord::ctrl(Y), "edit.redo"),
            Binding::new(Chord::ctrl(C), "edit.copy"),
            Binding::new(Chord::ctrl_shift(C), "edit.copy_pose"),
            Binding::new(Chord::ctrl(V), "edit.paste"),
            Binding::new(Chord::ctrl_shift(V), "edit.paste_mirrored"),
            Binding::new(Chord::ctrl(D), "edit.duplicate"),
            Binding::new(Chord::plain(F2), "edit.rename"),
            // ── Mode, keying, markers ───────────────────────────────────────
            Binding::new(Chord::plain(Tab), "mode.toggle"),
            Binding::new(Chord::plain(K), "anim.key_pose"),
            Binding::new(Chord::plain(M), "anim.add_marker"),
            // ── Tools ───────────────────────────────────────────────────────
            Binding::new(Chord::plain(V), "tool.select"),
            Binding::new(Chord::plain(B), "tool.create_bone"),
            Binding::new(Chord::plain(W), "tool.weight_paint"),
            // ── Transform gizmo ─────────────────────────────────────────────
            Binding::new(Chord::plain(T), "gizmo.translate"),
            Binding::new(Chord::plain(R), "gizmo.rotate"),
            Binding::new(Chord::plain(S), "gizmo.scale"),
            Binding::new(Chord::plain(H), "gizmo.shear"),
            // ── View ────────────────────────────────────────────────────────
            // Bare digits rather than a chord: these get flipped constantly
            // while rigging, and a chord is a chord too many.
            Binding::new(Chord::plain(Num1), "view.toggle_artwork"),
            Binding::new(Chord::plain(Num2), "view.toggle_bones"),
            Binding::new(Chord::ctrl(Comma), "app.settings"),
        ];
        Self { bindings }
    }

    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// Operator ids to fire for this frame's input, in table order.
    ///
    /// `typing` suppresses every binding that has not opted in with
    /// [`Binding::while_typing`] — otherwise naming a bone would scrub the
    /// timeline and swap the tool on the way past.
    ///
    /// Returns every match rather than the first: two operators may legitimately
    /// share a chord when at most one of them is `enabled` at a time, and the
    /// registry declines the rest. Resolving to one here would make which of
    /// them wins depend on table order instead of on applicability.
    pub fn resolve(&self, input: &egui::InputState, typing: bool) -> Vec<&str> {
        self.resolve_pressed(typing, &input.modifiers, |key| input.key_pressed(key))
    }

    /// [`resolve`](Self::resolve) with the input source as a closure.
    ///
    /// Split out so the suppression and matching rules can be tested against a
    /// synthetic key press. Driving them through a real `InputState` needs a
    /// live egui context, and a test that only inspected the table's flags would
    /// pass with this filter deleted.
    pub fn resolve_pressed(
        &self,
        typing: bool,
        mods: &Modifiers,
        mut pressed: impl FnMut(Key) -> bool,
    ) -> Vec<&str> {
        self.bindings
            .iter()
            .filter(|b| !typing || b.while_typing)
            .filter(|b| b.chord.matches(mods) && pressed(b.chord.key))
            .map(|b| b.operator.as_str())
            .collect()
    }

    /// The first chord bound to `operator`, for display next to a menu entry.
    pub fn chord_for(&self, operator: &str) -> Option<Chord> {
        self.bindings
            .iter()
            .find(|b| b.operator == operator)
            .map(|b| b.chord)
    }

    /// Bind `chord` to `operator`, replacing any binding that chord already had.
    ///
    /// One chord fires one operator; an operator may have several chords. That
    /// asymmetry is deliberate — `Ctrl+Y` and `Ctrl+Shift+Z` both mean redo, and
    /// a keymap editor that refused the second would be wrong.
    pub fn bind(&mut self, chord: Chord, operator: &str) {
        self.bindings.retain(|b| b.chord != chord);
        self.bindings.push(Binding::new(chord, operator));
    }

    /// Remove every binding for `chord`.
    pub fn unbind(&mut self, chord: Chord) {
        self.bindings.retain(|b| b.chord != chord);
    }

    /// Bindings naming an operator the registry does not have.
    ///
    /// For the settings UI to show as stale rather than for the loader to drop:
    /// a plugin that is merely not loaded right now must get its keys back when
    /// it returns.
    pub fn unresolved<'a>(
        &'a self,
        registry: &'a crate::commands::registry::Registry,
    ) -> impl Iterator<Item = &'a Binding> + 'a {
        self.bindings
            .iter()
            .filter(move |b| registry.get(&b.operator).is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::Registry;

    #[test]
    fn a_plain_chord_does_not_fire_under_a_modifier() {
        // The regression this whole design exists to prevent: `S` is Scale and
        // `Ctrl+S` is Save. The old inline handler guarded it with `!ctrl` by
        // hand, once, at the one site that needed it.
        let scale = Chord::plain(Key::S);
        assert!(scale.matches(&Modifiers::NONE));
        assert!(!scale.matches(&Modifiers::CTRL));
        assert!(!scale.matches(&Modifiers::SHIFT));
    }

    #[test]
    fn shift_h_and_h_are_different_bindings() {
        // Shift+H is viewport isolation (T-903); H is Shear. Exact matching is
        // what keeps one from tripping the other.
        let shear = Chord::plain(Key::H);
        assert!(!shear.matches(&Modifiers::SHIFT));
    }

    #[test]
    fn a_ctrl_chord_does_not_fire_bare() {
        let undo = Chord::ctrl(Key::Z);
        assert!(undo.matches(&Modifiers::CTRL));
        assert!(!undo.matches(&Modifiers::NONE));
        assert!(
            !undo.matches(&Modifiers::CTRL.plus(Modifiers::SHIFT)),
            "Ctrl+Shift+Z is redo, not undo"
        );
    }

    #[test]
    fn every_builtin_binding_names_a_real_operator() {
        // A shipped binding for a nonexistent id would be a dead key that no
        // test otherwise notices, since unknown ids resolve silently by design.
        let registry = Registry::with_builtins();
        let keymap = Keymap::builtin();
        let dead: Vec<_> = keymap.unresolved(&registry).map(|b| &b.operator).collect();
        assert!(dead.is_empty(), "bindings with no operator: {dead:?}");
    }

    #[test]
    fn no_two_builtin_bindings_share_a_chord() {
        // Sharing a chord is legal (see `resolve`), but among *built-ins* it
        // means two verbs race for one key, which is always a mistake.
        let keymap = Keymap::builtin();
        let mut seen = std::collections::HashSet::new();
        for binding in keymap.bindings() {
            assert!(
                seen.insert(binding.chord),
                "{} is bound twice",
                binding.chord.label()
            );
        }
    }

    #[test]
    fn rebinding_a_chord_displaces_the_old_operator() {
        let mut keymap = Keymap::builtin();
        let scale = Chord::plain(Key::S);
        assert_eq!(keymap.chord_for("gizmo.scale"), Some(scale));

        keymap.bind(scale, "tool.select");

        let bound: Vec<_> = keymap
            .bindings()
            .iter()
            .filter(|b| b.chord == scale)
            .map(|b| b.operator.as_str())
            .collect();
        assert_eq!(bound, ["tool.select"], "one chord, one operator");
        assert_eq!(
            keymap.chord_for("gizmo.scale"),
            None,
            "the displaced operator has no key left"
        );
    }

    #[test]
    fn an_operator_may_keep_several_chords() {
        // Ctrl+Y and Ctrl+Shift+Z both mean redo, and both must survive.
        let keymap = Keymap::builtin();
        let redo: Vec<_> = keymap
            .bindings()
            .iter()
            .filter(|b| b.operator == "edit.redo")
            .collect();
        assert_eq!(redo.len(), 2);
    }

    #[test]
    fn an_unknown_operator_id_survives_a_roundtrip() {
        // A plugin's binding must come back when the plugin does, so the loader
        // may not quietly drop what it cannot resolve today.
        let mut keymap = Keymap::builtin();
        keymap.bind(Chord::plain(Key::G), "someplugin.doathing");

        let json = serde_json::to_string(&keymap).expect("serialize");
        let back: Keymap = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(
            back.chord_for("someplugin.doathing"),
            Some(Chord::plain(Key::G))
        );

        let registry = Registry::with_builtins();
        let stale: Vec<_> = back
            .unresolved(&registry)
            .map(|b| b.operator.as_str())
            .collect();
        assert_eq!(
            stale,
            ["someplugin.doathing"],
            "reported as stale, not discarded"
        );
    }

    #[test]
    fn a_bare_letter_does_not_fire_into_a_text_field() {
        // Naming a bone types `s`, `k`, `b`, `w` — every one a bare binding. If
        // suppression is not applied, renaming a bone swaps the tool and scales
        // the selection on the way past.
        let keymap = Keymap::builtin();

        let idle = keymap.resolve_pressed(false, &Modifiers::NONE, |k| k == Key::S);
        assert_eq!(idle, ["gizmo.scale"], "S is Scale when nothing has focus");

        let typing = keymap.resolve_pressed(true, &Modifiers::NONE, |k| k == Key::S);
        assert!(typing.is_empty(), "S is a letter while typing, not a verb");
    }

    #[test]
    fn undo_still_fires_inside_a_text_field() {
        // The other half of the rule: suppressing everything would be as wrong
        // as suppressing nothing. Ctrl+Z in a field is undo.
        let keymap = Keymap::builtin();
        let fired = keymap.resolve_pressed(true, &Modifiers::CTRL, |k| k == Key::Z);
        assert_eq!(fired, ["edit.undo"]);
    }

    #[test]
    fn ctrl_s_resolves_to_nothing_rather_than_scale() {
        // The collision the old `!ctrl` guard existed to prevent, now structural.
        let keymap = Keymap::builtin();
        let fired = keymap.resolve_pressed(false, &Modifiers::CTRL, |k| k == Key::S);
        assert!(
            fired.is_empty(),
            "Ctrl+S is Save, handled outside the keymap"
        );
    }

    #[test]
    fn shift_h_does_not_reach_shear() {
        // Shift+H is viewport isolation (T-903). The bare-key match used to trip
        // Shear too, which is why the old handler carried `!shift`.
        let keymap = Keymap::builtin();
        let fired = keymap.resolve_pressed(false, &Modifiers::SHIFT, |k| k == Key::H);
        assert!(fired.is_empty());
    }

    #[test]
    fn the_menu_shortcut_is_the_one_that_actually_fires() {
        // Regression: the Edit menu hardcoded "Ctrl+Y" next to Redo while the
        // binding it advertised was Ctrl+Shift+Z, and nothing could notice
        // because the string and the handler were written out separately.
        // `chord_for` is what the menu now displays, so it must resolve.
        let keymap = Keymap::builtin();
        for operator in ["edit.undo", "edit.redo", "edit.copy", "app.settings"] {
            let chord = keymap
                .chord_for(operator)
                .unwrap_or_else(|| panic!("{operator} has no chord to display"));
            let fired = keymap.resolve_pressed(
                false,
                &Modifiers {
                    ctrl: chord.ctrl,
                    shift: chord.shift,
                    alt: chord.alt,
                    ..Modifiers::NONE
                },
                |k| k == chord.key,
            );
            assert!(
                fired.contains(&operator),
                "{operator} shows {} but that chord fires {fired:?}",
                chord.label()
            );
        }
    }

    #[test]
    fn chords_read_the_way_they_are_written() {
        assert_eq!(Chord::ctrl_shift(Key::S).label(), "Ctrl+Shift+S");
        assert_eq!(Chord::plain(Key::F2).label(), "F2");
        // egui's own name, not the glyph — punctuation keys read as `Comma`,
        // `Minus`, `Slash`. Left as-is so the keymap file and the label agree.
        assert_eq!(Chord::ctrl(Key::Comma).label(), "Ctrl+Comma");
    }
}
