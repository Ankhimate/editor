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

    /// Move `operator` to `chord`, giving up whatever keys it had.
    ///
    /// What a settings row does: the user pressed a new chord for this verb and
    /// expects the old one to stop working. Use [`add_binding`](Self::add_binding)
    /// for the other intent — a *second* key for the same verb.
    pub fn rebind(&mut self, chord: Chord, operator: &str) {
        self.bindings.retain(|b| b.operator != operator);
        self.add_binding(chord, operator);
    }

    /// Give `operator` an additional chord, keeping any it already has.
    ///
    /// One chord fires one operator, so this displaces whatever held `chord`.
    /// An operator may hold several chords, though — `Ctrl+Y` and `Ctrl+Shift+Z`
    /// both mean redo, and a keymap that refused the second would be wrong.
    ///
    /// Carries `while_typing` over from whatever the operator was already bound
    /// to. Rebinding undo must not quietly stop it working inside a text field:
    /// that flag is a property of the *verb*, not of the key it sits on, and a
    /// user moving `Ctrl+Z` to `Ctrl+U` has said nothing about text fields.
    pub fn add_binding(&mut self, chord: Chord, operator: &str) {
        let while_typing = self
            .bindings
            .iter()
            .find(|b| b.operator == operator)
            .map(|b| b.while_typing)
            .unwrap_or_else(|| {
                // No existing binding to inherit from — fall back to what the
                // built-in table says about this operator, so rebinding an
                // operator whose only key the user had cleared still behaves.
                Self::builtin()
                    .bindings
                    .iter()
                    .find(|b| b.operator == operator)
                    .is_some_and(|b| b.while_typing)
            });
        self.bindings.retain(|b| b.chord != chord);
        self.bindings.push(Binding {
            chord,
            operator: operator.to_string(),
            while_typing,
        });
    }

    /// Remove every binding for `chord`.
    pub fn unbind(&mut self, chord: Chord) {
        self.bindings.retain(|b| b.chord != chord);
    }

    /// Restore `operator` to the keys the built-in table gives it.
    ///
    /// Per-operator rather than whole-table: someone who has rebound six things
    /// and wants the seventh back should not lose the six.
    pub fn reset(&mut self, operator: &str) {
        self.bindings.retain(|b| b.operator != operator);
        for binding in Self::builtin().bindings {
            if binding.operator == operator {
                // Displace whatever currently holds the default chord, or the
                // reset would leave two operators racing for one key.
                self.bindings.retain(|b| b.chord != binding.chord);
                self.bindings.push(binding);
            }
        }
    }

    /// Adopt built-in bindings for operators this table has never heard of.
    ///
    /// The whole table is serialized, so a config written before a binding
    /// existed would otherwise never see it — the new key would be dead for
    /// every existing user and live only for fresh installs, which is the kind
    /// of difference nobody reproduces.
    ///
    /// Keyed on the *operator*, not the chord: an operator the user has bound
    /// somewhere else is not missing, and re-adding its default would hand them
    /// a second key they never asked for. An operator they deliberately cleared
    /// does come back, which is the one case this gets wrong; it is the safer
    /// side of the trade, since the alternative loses a new feature silently.
    ///
    /// Returns how many were added, for the log.
    pub fn merge_new_defaults(&mut self) -> usize {
        let known: std::collections::HashSet<&str> =
            self.bindings.iter().map(|b| b.operator.as_str()).collect();
        let missing: Vec<Binding> = Self::builtin()
            .bindings
            .into_iter()
            .filter(|b| !known.contains(b.operator.as_str()))
            .collect();
        let mut added = 0;
        for binding in missing {
            // Never over a chord the user has already spoken for. Counted after
            // this check, not before, so the number reported is what landed.
            if self.bindings.iter().any(|b| b.chord == binding.chord) {
                continue;
            }
            self.bindings.push(binding);
            added += 1;
        }
        added
    }

    /// The operator `chord` currently fires, if any. For conflict highlighting.
    pub fn operator_for(&self, chord: Chord) -> Option<&str> {
        self.bindings
            .iter()
            .find(|b| b.chord == chord)
            .map(|b| b.operator.as_str())
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

        keymap.rebind(scale, "tool.select");

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
        keymap.rebind(Chord::plain(Key::G), "someplugin.doathing");

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
    fn rebinding_undo_keeps_it_working_in_a_text_field() {
        // `while_typing` belongs to the verb, not the key. A user moving undo to
        // Ctrl+U has said nothing about text fields, and silently losing undo
        // there would be a strange punishment for rebinding a key.
        let mut keymap = Keymap::builtin();
        keymap.rebind(Chord::ctrl(Key::U), "edit.undo");

        let fired = keymap.resolve_pressed(true, &Modifiers::CTRL, |k| k == Key::U);
        assert_eq!(fired, ["edit.undo"], "still fires while typing");
    }

    #[test]
    fn rebinding_a_bare_key_does_not_gain_text_field_powers() {
        // The other direction: inheriting must not turn a tool key into one that
        // fires mid-rename.
        let mut keymap = Keymap::builtin();
        keymap.rebind(Chord::plain(Key::G), "gizmo.scale");

        let typing = keymap.resolve_pressed(true, &Modifiers::NONE, |k| k == Key::G);
        assert!(typing.is_empty());
    }

    #[test]
    fn a_new_builtin_reaches_a_config_that_predates_it() {
        // The hazard of serializing the whole table: a binding added after a
        // user's config was written would be live for fresh installs and dead
        // for everyone else.
        let mut old = Keymap::builtin();
        old.bindings.retain(|b| b.operator != "anim.add_marker");
        assert!(old.chord_for("anim.add_marker").is_none());

        let added = old.merge_new_defaults();

        assert_eq!(added, 1);
        assert_eq!(old.chord_for("anim.add_marker"), Some(Chord::plain(Key::M)));
    }

    #[test]
    fn merging_does_not_second_guess_a_rebinding() {
        // An operator the user moved is not missing. Re-adding its default would
        // hand them a second key they never asked for.
        let mut keymap = Keymap::builtin();
        keymap.unbind(Chord::plain(Key::M));
        keymap.rebind(Chord::plain(Key::G), "anim.add_marker");

        assert_eq!(keymap.merge_new_defaults(), 0);
        assert_eq!(
            keymap.operator_for(Chord::plain(Key::M)),
            None,
            "the vacated default stays vacant"
        );
    }

    #[test]
    fn merging_does_not_steal_a_chord_the_user_took() {
        // `M` reassigned to something else, and the marker operator cleared. The
        // marker's default must not evict the user's binding.
        let mut keymap = Keymap::builtin();
        keymap.bindings.retain(|b| b.operator != "anim.add_marker");
        keymap.rebind(Chord::plain(Key::M), "tool.select");

        keymap.merge_new_defaults();

        assert_eq!(
            keymap.operator_for(Chord::plain(Key::M)),
            Some("tool.select")
        );
    }

    #[test]
    fn reset_restores_one_operator_without_disturbing_the_rest() {
        let mut keymap = Keymap::builtin();
        keymap.rebind(Chord::plain(Key::G), "gizmo.scale");
        keymap.rebind(Chord::plain(Key::J), "tool.select");
        assert_eq!(keymap.chord_for("gizmo.scale"), Some(Chord::plain(Key::G)));

        keymap.reset("gizmo.scale");

        assert_eq!(
            keymap.chord_for("gizmo.scale"),
            Some(Chord::plain(Key::S)),
            "back to its default"
        );
        assert_eq!(
            keymap.operator_for(Chord::plain(Key::G)),
            None,
            "and the key it borrowed is free again"
        );
        assert_eq!(
            keymap.chord_for("tool.select"),
            Some(Chord::plain(Key::J)),
            "the other rebinding survived"
        );
    }

    #[test]
    fn reset_displaces_whoever_holds_the_default_chord() {
        // Otherwise the reset leaves two operators racing for one key, and which
        // wins depends on table order.
        let mut keymap = Keymap::builtin();
        keymap.rebind(Chord::plain(Key::S), "tool.select");
        keymap.reset("gizmo.scale");

        assert_eq!(
            keymap.operator_for(Chord::plain(Key::S)),
            Some("gizmo.scale")
        );
        let racers = keymap
            .bindings()
            .iter()
            .filter(|b| b.chord == Chord::plain(Key::S))
            .count();
        assert_eq!(racers, 1);
    }

    #[test]
    fn a_config_roundtrips_through_json_with_its_rebindings() {
        let mut keymap = Keymap::builtin();
        keymap.rebind(Chord::ctrl(Key::U), "edit.undo");

        let json = serde_json::to_string(&keymap).expect("serialize");
        let back: Keymap = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.chord_for("edit.undo"), Some(Chord::ctrl(Key::U)));
        let fired = back.resolve_pressed(true, &Modifiers::CTRL, |k| k == Key::U);
        assert_eq!(fired, ["edit.undo"], "while_typing survived the file");
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
