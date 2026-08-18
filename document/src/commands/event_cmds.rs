//! Animation event authoring as undoable commands (T-506).
//!
//! Events are animation data, not rig structure, so unlike constraints these are
//! **Animate-only**: an event marks a moment in a clip, and there is no clip in
//! Setup mode to mark.
//!
//! Each command snapshots the clip's event list rather than inverting its edit.
//! The list is a handful of small structs, and the alternative — index-based
//! inverses that survive a re-sort — is where the bugs would be.

use super::EditCommand;
use crate::WorkMode;
use crate::doc::Document;
use ankhimate_core::animation::EventKey;
use ankhimate_core::ids::AnimationId;

fn events_mut(doc: &mut Document, anim: AnimationId) -> Option<&mut Vec<EventKey>> {
    doc.animations.get_mut(anim).map(|a| &mut a.events)
}

/// Add an event at a time.
pub struct AddEvent {
    anim: AnimationId,
    event: EventKey,
    before: Option<Vec<EventKey>>,
}

impl AddEvent {
    pub fn new(anim: AnimationId, name: impl Into<String>, time: f32) -> Self {
        Self {
            anim,
            event: EventKey {
                time: time.max(0.0),
                name: name.into(),
                int_value: 0,
                float_value: 0.0,
                string_value: String::new(),
                audio: String::new(),
                volume: 1.0,
                balance: 0.0,
            },
            before: None,
        }
    }
}

impl EditCommand for AddEvent {
    fn apply(&mut self, doc: &mut Document) {
        let Some(events) = events_mut(doc, self.anim) else {
            return;
        };
        self.before = Some(events.clone());
        events.push(self.event.clone());
        // Kept in time order so the dopesheet and the runtime agree on which
        // event is "the next one" without either having to sort.
        events.sort_by(|a, b| a.time.total_cmp(&b.time));
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(events)) = (self.before.take(), events_mut(doc, self.anim)) {
            *events = before;
        }
    }

    fn label(&self) -> &str {
        "Add Event"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Animate)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Copy an event, payload and all.
///
/// A separate command rather than add-then-edit: two commands would be two undo
/// steps for one intention, and the intermediate state — a copy with the right
/// name and a blank payload — is not one anybody asked to see.
pub struct DuplicateEvent {
    anim: AnimationId,
    index: usize,
    before: Option<Vec<EventKey>>,
}

impl DuplicateEvent {
    pub fn new(anim: AnimationId, index: usize) -> Self {
        Self {
            anim,
            index,
            before: None,
        }
    }
}

impl EditCommand for DuplicateEvent {
    fn apply(&mut self, doc: &mut Document) {
        let Some(events) = events_mut(doc, self.anim) else {
            return;
        };
        let Some(source) = events.get(self.index).cloned() else {
            return;
        };
        self.before = Some(events.clone());
        // Same time, so it lands next to its original rather than somewhere the
        // user has to go looking for.
        events.push(source);
        events.sort_by(|a, b| a.time.total_cmp(&b.time));
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(events)) = (self.before.take(), events_mut(doc, self.anim)) {
            *events = before;
        }
    }

    fn label(&self) -> &str {
        "Duplicate Event"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Animate)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// What an edit does to one event.
pub enum EventEdit {
    /// Move it in time. Merges, so dragging a marker is one step.
    SetTime(f32),
    Rename(String),
    SetPayload {
        int_value: i32,
        float_value: f32,
        string_value: String,
        /// Asset name of the sound to fire with the event; empty is silent.
        audio: String,
        volume: f32,
        balance: f32,
    },
    Remove,
}

/// Apply an [`EventEdit`] to the event at `index`.
pub struct EditEvent {
    anim: AnimationId,
    index: usize,
    edit: EventEdit,
    before: Option<Vec<EventKey>>,
    label: &'static str,
}

impl EditEvent {
    pub fn new(anim: AnimationId, index: usize, edit: EventEdit) -> Self {
        let label = match &edit {
            EventEdit::SetTime(_) => "Move Event",
            EventEdit::Rename(_) => "Rename Event",
            EventEdit::SetPayload { .. } => "Edit Event",
            EventEdit::Remove => "Delete Event",
        };
        Self {
            anim,
            index,
            edit,
            before: None,
            label,
        }
    }
}

impl EditCommand for EditEvent {
    fn apply(&mut self, doc: &mut Document) {
        let Some(events) = events_mut(doc, self.anim) else {
            return;
        };
        if self.index >= events.len() {
            return;
        }
        if self.before.is_none() {
            self.before = Some(events.clone());
        }
        match &self.edit {
            EventEdit::SetTime(time) => {
                events[self.index].time = time.max(0.0);
                events.sort_by(|a, b| a.time.total_cmp(&b.time));
            }
            EventEdit::Rename(name) => events[self.index].name = name.clone(),
            EventEdit::SetPayload {
                int_value,
                float_value,
                string_value,
                audio,
                volume,
                balance,
            } => {
                let e = &mut events[self.index];
                e.int_value = *int_value;
                e.float_value = *float_value;
                e.string_value = string_value.clone();
                e.audio = audio.clone();
                e.volume = *volume;
                e.balance = *balance;
            }
            EventEdit::Remove => {
                events.remove(self.index);
            }
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(events)) = (self.before.take(), events_mut(doc, self.anim)) {
            *events = before;
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<EditEvent>() else {
            return false;
        };
        if other.anim != self.anim {
            return false;
        }
        // Only a drag merges — and only with itself. A re-sort can change the
        // index mid-drag, so merging on index equality would stitch together
        // edits to two different events.
        match (&mut self.edit, &other.edit) {
            (EventEdit::SetTime(ours), EventEdit::SetTime(theirs)) => {
                *ours = *theirs;
                self.index = other.index;
                true
            }
            _ => false,
        }
    }

    fn label(&self) -> &str {
        self.label
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Animate)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::History;
    use ankhimate_core::animation::Animation;

    fn doc_with_clip() -> (Document, AnimationId) {
        let mut doc = Document::new();
        let anim = doc.animations.insert(Animation {
            name: "walk".into(),
            duration: 1.0,
            looping: true,
            timelines: Vec::new(),
            events: Vec::new(),
            markers: Vec::new(),
            bone_offsets: Vec::new(),
        });
        (doc, anim)
    }

    fn times(doc: &Document, anim: AnimationId) -> Vec<f32> {
        doc.animations[anim].events.iter().map(|e| e.time).collect()
    }

    #[test]
    fn events_are_kept_in_time_order() {
        let (mut doc, anim) = doc_with_clip();
        let mut history = History::default();
        for (name, time) in [("late", 0.8), ("early", 0.2), ("mid", 0.5)] {
            history.push(Box::new(AddEvent::new(anim, name, time)), &mut doc);
        }
        assert_eq!(times(&doc, anim), vec![0.2, 0.5, 0.8]);
    }

    #[test]
    fn adding_and_undoing_leaves_the_list_as_it_was() {
        let (mut doc, anim) = doc_with_clip();
        let mut history = History::default();
        history.push(Box::new(AddEvent::new(anim, "step", 0.5)), &mut doc);
        history.undo(&mut doc);
        assert!(doc.animations[anim].events.is_empty());
    }

    #[test]
    fn dragging_a_marker_is_one_undo_step() {
        let (mut doc, anim) = doc_with_clip();
        let mut history = History::default();
        history.push(Box::new(AddEvent::new(anim, "step", 0.5)), &mut doc);
        for t in [0.6, 0.7, 0.9] {
            history.push(
                Box::new(EditEvent::new(anim, 0, EventEdit::SetTime(t))),
                &mut doc,
            );
        }
        history.undo(&mut doc);
        assert_eq!(
            times(&doc, anim),
            vec![0.5],
            "one undo returns to before the whole drag"
        );
    }

    /// A drag that crosses another marker re-sorts the list. The command has to
    /// undo to the *list*, not to an index that now means a different event.
    #[test]
    fn dragging_past_another_event_still_undoes_correctly() {
        let (mut doc, anim) = doc_with_clip();
        let mut history = History::default();
        history.push(Box::new(AddEvent::new(anim, "a", 0.2)), &mut doc);
        history.push(Box::new(AddEvent::new(anim, "b", 0.6)), &mut doc);
        // Drag "a" past "b".
        history.push(
            Box::new(EditEvent::new(anim, 0, EventEdit::SetTime(0.9))),
            &mut doc,
        );
        assert_eq!(times(&doc, anim), vec![0.6, 0.9]);
        assert_eq!(doc.animations[anim].events[1].name, "a");

        history.undo(&mut doc);
        let names: Vec<&str> = doc.animations[anim]
            .events
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(names, vec!["a", "b"], "both events back where they started");
        assert_eq!(times(&doc, anim), vec![0.2, 0.6]);
    }

    #[test]
    fn removing_an_event_undoes_to_the_same_place_in_the_order() {
        let (mut doc, anim) = doc_with_clip();
        let mut history = History::default();
        for (name, time) in [("a", 0.2), ("b", 0.5), ("c", 0.8)] {
            history.push(Box::new(AddEvent::new(anim, name, time)), &mut doc);
        }
        history.push(
            Box::new(EditEvent::new(anim, 1, EventEdit::Remove)),
            &mut doc,
        );
        assert_eq!(times(&doc, anim), vec![0.2, 0.8]);

        history.undo(&mut doc);
        let names: Vec<&str> = doc.animations[anim]
            .events
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }
}
