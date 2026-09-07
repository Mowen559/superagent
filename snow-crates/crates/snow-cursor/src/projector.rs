use std::collections::HashSet;

use crate::model::{
    AttachedCursorSample, CursorShapeCapture, CursorShapeState, CursorSnapshot, CursorTargetInfo,
};

/// Converts absolute cursor snapshots into target-relative samples and
/// deduplicates repeated cursor shapes into cached references.
#[derive(Default)]
pub struct CursorProjector {
    emitted_shape_ids: HashSet<crate::CursorShapeId>,
}

impl CursorProjector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn project(
        &mut self,
        target: &CursorTargetInfo,
        snapshot: CursorSnapshot,
    ) -> AttachedCursorSample {
        let x = snapshot.absolute_x - target.origin_x;
        let y = snapshot.absolute_y - target.origin_y;
        let visible = snapshot.visible && target.contains_relative(x, y);
        let shape = match snapshot.shape {
            CursorShapeCapture::Captured(shape) => {
                if self.emitted_shape_ids.insert(shape.shape_id) {
                    CursorShapeState::Embedded(shape)
                } else {
                    CursorShapeState::Cached(shape.shape_id)
                }
            }
            CursorShapeCapture::Unavailable => CursorShapeState::Unavailable,
        };

        AttachedCursorSample {
            x,
            y,
            visible,
            shape,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CursorCompositionMode, CursorShape};

    fn sample_shape(seed: u8) -> CursorShape {
        CursorShape::from_rgba(
            0,
            0,
            2,
            2,
            CursorCompositionMode::AlphaBlend,
            vec![seed; 16],
        )
    }

    #[test]
    fn first_shape_is_embedded_then_cached() {
        let mut projector = CursorProjector::new();
        let target = CursorTargetInfo {
            origin_x: 10,
            origin_y: 20,
            width: 100,
            height: 100,
        };
        let shape = sample_shape(7);

        let first = projector.project(
            &target,
            CursorSnapshot {
                absolute_x: 11,
                absolute_y: 21,
                visible: true,
                shape: CursorShapeCapture::Captured(shape.clone()),
            },
        );
        let second = projector.project(
            &target,
            CursorSnapshot {
                absolute_x: 12,
                absolute_y: 22,
                visible: true,
                shape: CursorShapeCapture::Captured(shape.clone()),
            },
        );

        assert!(matches!(first.shape, CursorShapeState::Embedded(_)));
        assert!(matches!(
            second.shape,
            CursorShapeState::Cached(id) if id == shape.shape_id
        ));
    }

    #[test]
    fn unavailable_shape_stays_unavailable() {
        let mut projector = CursorProjector::new();
        let target = CursorTargetInfo {
            origin_x: 0,
            origin_y: 0,
            width: 100,
            height: 100,
        };

        let sample = projector.project(
            &target,
            CursorSnapshot {
                absolute_x: 30,
                absolute_y: 40,
                visible: true,
                shape: CursorShapeCapture::Unavailable,
            },
        );

        assert!(matches!(sample.shape, CursorShapeState::Unavailable));
        assert_eq!(sample.shape_id(), None);
    }

    #[test]
    fn projection_marks_out_of_bounds_cursor_invisible() {
        let mut projector = CursorProjector::new();
        let target = CursorTargetInfo {
            origin_x: 100,
            origin_y: 100,
            width: 50,
            height: 50,
        };
        let shape = sample_shape(1);

        let sample = projector.project(
            &target,
            CursorSnapshot {
                absolute_x: 20,
                absolute_y: 30,
                visible: true,
                shape: CursorShapeCapture::Captured(shape.clone()),
            },
        );

        assert!(!sample.visible);
        assert_eq!(sample.x, -80);
        assert_eq!(sample.y, -70);
        assert_eq!(sample.shape_id(), Some(shape.shape_id));
    }
}
