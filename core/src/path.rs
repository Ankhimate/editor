//! Path sampling for path attachments and constraints (T-502).
//!
//! A path is a polyline (bezier control points are authored, but the runtime
//! contract is the flattened polyline — a curve nobody can measure is not
//! useful to a constraint). The one hard part is **constant speed**: bones
//! spaced evenly along a path must be evenly spaced in *distance*, not in
//! parameter, or a chain bunches up wherever the curve is tight.
//!
//! So sampling goes through an arc-length table: cumulative distance per
//! segment, then a binary search to turn "60% of the way along" into a point.

use glam::Vec2;

/// A path flattened for measurement, with its arc-length table.
#[derive(Debug, Clone, Default)]
pub struct SampledPath {
    /// Points in world space, in order.
    pub points: Vec<Vec2>,
    /// Cumulative distance to each point; `lengths[0]` is always `0`.
    lengths: Vec<f32>,
    pub closed: bool,
}

impl SampledPath {
    /// Build the arc-length table for a polyline.
    pub fn new(points: Vec<Vec2>, closed: bool) -> Self {
        let mut lengths = Vec::with_capacity(points.len() + 1);
        let mut total = 0.0;
        lengths.push(0.0);
        for i in 1..points.len() {
            total += (points[i] - points[i - 1]).length();
            lengths.push(total);
        }
        // A closed path's last segment runs back to the start, and it is a real
        // segment: without it, positions past the last point would clamp instead
        // of wrapping.
        if closed && points.len() > 1 {
            total += (points[0] - points[points.len() - 1]).length();
            lengths.push(total);
        }
        Self {
            points,
            lengths,
            closed,
        }
    }

    /// Total arc length.
    pub fn length(&self) -> f32 {
        self.lengths.last().copied().unwrap_or(0.0)
    }

    /// The point at `distance` along the path, and the direction there.
    ///
    /// Past the end: a closed path wraps, an open one clamps and keeps the end
    /// direction — a bone pushed past the tip should keep pointing the way the
    /// path was going, not snap to some default.
    pub fn at(&self, distance: f32) -> Option<(Vec2, f32)> {
        if self.points.len() < 2 {
            return self.points.first().map(|p| (*p, 0.0));
        }
        let total = self.length();
        if total <= 1e-6 {
            return self.points.first().map(|p| (*p, 0.0));
        }
        let d = if self.closed {
            distance.rem_euclid(total)
        } else {
            distance.clamp(0.0, total)
        };

        // Binary search the cumulative table: which segment contains `d`.
        let segment = match self.lengths.binary_search_by(|probe| probe.total_cmp(&d)) {
            Ok(i) => i.min(self.lengths.len() - 2),
            Err(i) => i.saturating_sub(1).min(self.lengths.len() - 2),
        };

        let from = self.points[segment % self.points.len()];
        let to = self.points[(segment + 1) % self.points.len()];
        let span = (self.lengths[segment + 1] - self.lengths[segment]).max(1e-6);
        let t = ((d - self.lengths[segment]) / span).clamp(0.0, 1.0);

        let direction = to - from;
        let angle = if direction.length_squared() > 1e-12 {
            direction.y.atan2(direction.x)
        } else {
            0.0
        };
        Some((from.lerp(to, t), angle))
    }

    /// `count` positions spread along the path, each with its direction.
    ///
    /// `position` slides the whole set along (0..1 of the path), `spacing`
    /// scales the gap between them. Even spacing is in **distance**, which is
    /// the entire reason for the arc-length table: parameter-spaced bones bunch
    /// up wherever the curve is tight.
    pub fn spread(&self, count: usize, position: f32, spacing: f32) -> Vec<(Vec2, f32)> {
        if count == 0 {
            return Vec::new();
        }
        let total = self.length();
        let start = position * total;
        // One bone sits at the position; several share the path evenly unless
        // `spacing` says otherwise.
        let gap = if count > 1 {
            total / (count - 1).max(1) as f32 * spacing
        } else {
            0.0
        };
        (0..count)
            .filter_map(|i| self.at(start + gap * i as f32))
            .collect()
    }

    /// `count` positions spread by **vertex index** rather than by distance.
    ///
    /// What `constant_speed = false` means: samples land in proportion to how
    /// many vertices they pass, so a densely-vertexed section of the curve gets
    /// more bones. Occasionally wanted — it is how you make a chain crowd around
    /// a detailed corner — and it is the honest opposite of the arc-length path
    /// rather than a second name for it.
    pub fn spread_by_index(&self, count: usize, position: f32, spacing: f32) -> Vec<(Vec2, f32)> {
        if count == 0 || self.points.len() < 2 {
            return Vec::new();
        }
        let last = if self.closed {
            self.points.len()
        } else {
            self.points.len() - 1
        } as f32;
        let start = position * last;
        let gap = if count > 1 {
            last / (count - 1).max(1) as f32 * spacing
        } else {
            0.0
        };
        (0..count)
            .map(|i| {
                let t = start + gap * i as f32;
                let t = if self.closed {
                    t.rem_euclid(last)
                } else {
                    t.clamp(0.0, last)
                };
                let index = (t.floor() as usize).min(self.points.len() - 1);
                let next = (index + 1) % self.points.len();
                let (from, to) = (self.points[index], self.points[next]);
                let direction = to - from;
                let angle = if direction.length_squared() > 1e-12 {
                    direction.y.atan2(direction.x)
                } else {
                    0.0
                };
                (from.lerp(to, t - t.floor()), angle)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An L-shape: 10 right, then 10 up. Total length 20, and the corner is at
    /// exactly half — which parameter-space sampling would also get right, so
    /// the uneven case below is the real test.
    fn corner() -> SampledPath {
        SampledPath::new(
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(10.0, 0.0),
                Vec2::new(10.0, 10.0),
            ],
            false,
        )
    }

    /// The flag has to mean something: on a path whose segments differ wildly,
    /// index spacing and distance spacing must not agree.
    #[test]
    fn index_spacing_differs_from_distance_spacing() {
        let path = SampledPath::new(
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(90.0, 0.0),
                Vec2::new(100.0, 0.0),
            ],
            false,
        );
        let by_distance = path.spread(3, 0.0, 1.0);
        let by_index = path.spread_by_index(3, 0.0, 1.0);
        assert!((by_distance[1].0.x - 50.0).abs() < 1e-3, "half the length");
        assert!(
            (by_index[1].0.x - 90.0).abs() < 1e-3,
            "the middle *vertex*, not the middle distance: {:?}",
            by_index[1].0
        );
    }

    #[test]
    fn arc_length_measures_the_polyline() {
        assert!((corner().length() - 20.0).abs() < 1e-4);
    }

    #[test]
    fn sampling_walks_at_constant_speed() {
        let path = corner();
        let (p, _) = path.at(5.0).unwrap();
        assert!((p - Vec2::new(5.0, 0.0)).length() < 1e-4, "{p:?}");
        let (p, _) = path.at(15.0).unwrap();
        assert!((p - Vec2::new(10.0, 5.0)).length() < 1e-4, "{p:?}");
    }

    /// The acceptance case: uneven segments must not bunch the samples. A path
    /// whose first segment is nine times the second would, in parameter space,
    /// put half the bones in the short one.
    #[test]
    fn uneven_segments_still_space_evenly_by_distance() {
        let path = SampledPath::new(
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(90.0, 0.0),
                Vec2::new(100.0, 0.0),
            ],
            false,
        );
        let spread = path.spread(5, 0.0, 1.0);
        let xs: Vec<f32> = spread.iter().map(|(p, _)| p.x).collect();
        for (i, x) in xs.iter().enumerate() {
            let expected = i as f32 * 25.0;
            assert!(
                (x - expected).abs() < 1e-3,
                "sample {i} at {x}, expected {expected}: {xs:?}"
            );
        }
    }

    #[test]
    fn direction_follows_the_segment() {
        let path = corner();
        let (_, angle) = path.at(2.0).unwrap();
        assert!(angle.abs() < 1e-4, "along +X: {angle}");
        let (_, angle) = path.at(15.0).unwrap();
        assert!(
            (angle - std::f32::consts::FRAC_PI_2).abs() < 1e-4,
            "along +Y: {angle}"
        );
    }

    #[test]
    fn a_closed_path_wraps_instead_of_clamping() {
        let square = SampledPath::new(
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(10.0, 0.0),
                Vec2::new(10.0, 10.0),
                Vec2::new(0.0, 10.0),
            ],
            true,
        );
        assert!(
            (square.length() - 40.0).abs() < 1e-4,
            "the closing edge counts"
        );
        let (start, _) = square.at(0.0).unwrap();
        let (wrapped, _) = square.at(40.0).unwrap();
        assert!(
            (start - wrapped).length() < 1e-3,
            "a full lap returns to the start: {start:?} vs {wrapped:?}"
        );
    }

    #[test]
    fn an_open_path_clamps_past_its_ends() {
        let path = corner();
        let (past, _) = path.at(1000.0).unwrap();
        assert!((past - Vec2::new(10.0, 10.0)).length() < 1e-3);
        let (before, _) = path.at(-5.0).unwrap();
        assert!((before - Vec2::new(0.0, 0.0)).length() < 1e-3);
    }

    #[test]
    fn position_slides_the_whole_chain_along() {
        let path = SampledPath::new(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], false);
        let at_start = path.spread(3, 0.0, 0.5);
        let moved = path.spread(3, 0.25, 0.5);
        for (i, ((a, _), (b, _))) in at_start.iter().zip(moved.iter()).enumerate() {
            assert!(
                (b.x - a.x - 25.0).abs() < 1e-3,
                "sample {i} slid {} instead of 25",
                b.x - a.x
            );
        }
    }

    #[test]
    fn a_degenerate_path_does_not_panic() {
        let empty = SampledPath::new(Vec::new(), false);
        assert!(empty.at(1.0).is_none());
        assert_eq!(empty.length(), 0.0);

        let single = SampledPath::new(vec![Vec2::new(3.0, 4.0)], true);
        assert_eq!(single.at(99.0).map(|(p, _)| p), Some(Vec2::new(3.0, 4.0)));

        let coincident = SampledPath::new(vec![Vec2::ZERO, Vec2::ZERO], false);
        assert!(coincident.at(1.0).is_some());
    }
}
