//! Manually driven opacity replay for performance experiments, not an animation API.
//! Adapted from Mesa Hills Research's Apache-2.0 paint-effect records:
//! mesa-hills-research/gpui@eb63578acb9637ba72146dfafc07e4e1cb03d16b.
//! Scheduling, general transforms, shader layers and native surfaces are outside this prototype.

use super::*;

#[derive(Clone)]
pub(super) struct OpacityRecord {
    ops: Range<usize>,
    value: Rc<Cell<f32>>,
}

impl Window {
    pub fn experimental_frame_snapshot(&self) -> String {
        let scene = &self.rendered_frame.scene;
        let hitboxes: Vec<_> = self
            .rendered_frame
            .hitboxes
            .iter()
            .map(|hitbox| (hitbox.bounds, hitbox.content_mask, hitbox.behavior))
            .collect();
        format!(
            "{:?}",
            (
                &scene.quads,
                &scene.shadows,
                &scene.paths,
                &scene.underlines,
                &scene.monochrome_sprites,
                &scene.subpixel_sprites,
                &scene.polychrome_sprites,
                scene.window_corner_mask,
                hitboxes
            )
        )
    }

    pub fn with_experimental_paint_opacity<R>(
        &mut self,
        value: Rc<Cell<f32>>,
        paint: impl FnOnce(&mut Window) -> R,
    ) -> R {
        self.invalidator.debug_assert_paint();
        let start = self.next_frame.scene.len();
        let index = self.next_frame.experimental_effects.len();
        self.next_frame.experimental_effects.push(OpacityRecord {
            ops: start..start,
            value: value.clone(),
        });
        self.next_frame.scene.push_experimental_opacity(value.get());
        let result = paint(self);
        self.next_frame.scene.pop_experimental_opacity();
        self.next_frame.experimental_effects[index].ops.end = self.next_frame.scene.len();
        result
    }

    pub(super) fn reuse_experimental_scene(&mut self, range: &Range<PaintIndex>) {
        let target = self.next_frame.scene.len();
        let records = range.start.experimental_effects_index..range.end.experimental_effects_index;
        let mut boundaries = SmallVec::<[(usize, bool, usize, f32); 8]>::new();
        for (index, record) in self.rendered_frame.experimental_effects[records]
            .iter()
            .enumerate()
        {
            let mut moved = record.clone();
            moved.ops = target + record.ops.start - range.start.scene_index
                ..target + record.ops.end - range.start.scene_index;
            self.next_frame.experimental_effects.push(moved);
            if !record.ops.is_empty() {
                boundaries.push((record.ops.start, true, index, record.value.get()));
                boundaries.push((record.ops.end, false, usize::MAX - index, 1.0));
            }
        }
        boundaries.sort_unstable_by_key(|&(at, starts, order, _)| (at, starts, order));
        let mut from = range.start.scene_index;
        for (at, starts, _, opacity) in boundaries {
            self.next_frame
                .scene
                .replay(from..at, &self.rendered_frame.scene);
            from = at;
            if starts {
                self.next_frame.scene.push_experimental_opacity(opacity);
            } else {
                self.next_frame.scene.pop_experimental_opacity();
            }
        }
        self.next_frame
            .scene
            .replay(from..range.end.scene_index, &self.rendered_frame.scene);
    }
}
