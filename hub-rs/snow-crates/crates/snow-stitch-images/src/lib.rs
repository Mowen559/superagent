mod compositor;
mod error;
mod estimator;
mod frame;
mod orb;
mod region;
mod state;
mod stitcher;
mod tiled_canvas;
mod trace;
mod types;

pub use compositor::{
    append_and_repaint, band_height, prepend_and_repaint, synthesize_append, synthesize_prepend,
};
pub use error::StitchError;
pub use estimator::VerticalMotionEstimator;
pub use frame::{Frame, Geometry, PixelFormat};
pub use state::{ViewportState, ViewportTransition};
pub use stitcher::{StitchResult, Stitcher, stitch, stitch_files, stitch_iter};
pub use tiled_canvas::{CANVAS_TILE_ROWS, CANVAS_TILE_SPAN, TiledCanvas, TiledCanvasSnapshot};
pub use trace::{StitchTraceEvent, TraceState};
pub use types::{
    CandidateDiagnostics, MotionDiagnostics, MotionEstimate, MotionEstimatorOptions, MotionOutcome,
    MotionStage, ReferenceMode, RegionDiagnostics, StitchAxis, StitchBranch, StitchOptions,
};
