use std::sync::mpsc::Sender;
use std::sync::Arc;
use windows::Win32::Foundation::{POINT, RECT};
use snow_ui_selector::ElementRegionService;
use crate::services::visual_pipeline::VisualPipelineRuntime;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    None,
    Copy,
    Save,
    Pin,
    Ocr,
    Translate,
    TTS,
    Record,
    RecordPause,
    RecordResume,
    RecordStop,
    RecordDiscard,
    Accept,
    Cancel,
    DrawPen,
    DrawLine,
    CopyText,
    CopyPath,
    OpenFolder,
    DrawRect,
    DrawArrow,
    DrawEllipse,
    DrawText,
    Mosaic,
    Eraser,
    Undo,
    Redo,
    ColorPick(u32),
    Thickness(i32),
}

impl Action {
    pub fn match_color(&self) -> Option<u32> {
        match self {
            Action::ColorPick(c) => Some(*c),
            _ => None,
        }
    }
}

pub struct CroppedResult {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub action: Action,
    pub final_pixels: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppMode {
    Selecting,
    ToolbarActive,
    DrawingPen,
    DrawingLine,
    Saved,
    DrawingRect,
    DrawingArrow,
    DrawingEllipse,
    DrawingText,
    DrawingMosaic,
    SelectingText,
    Recording,
}

#[derive(Clone, Debug)]
pub struct StrokeData {
    pub points: Vec<POINT>,
    pub color: u32,
    pub thickness: i32,
    pub is_eraser: bool,
}

#[derive(Clone, Debug)]
pub struct RectData {
    pub rect: RECT,
    pub color: u32,
    pub thickness: i32,
}

#[derive(Clone, Debug)]
pub struct ArrowData {
    pub start: POINT,
    pub end: POINT,
    pub color: u32,
    pub thickness: i32,
}

#[derive(Clone, Debug)]
pub struct EllipseData {
    pub rect: RECT,
    pub color: u32,
    pub thickness: i32,
}

#[derive(Clone, Debug)]
pub struct TextData {
    pub pos: POINT,
    pub text: String,
    pub color: u32,
    pub font_size: i32,
}

#[derive(Clone, Debug)]
pub struct MosaicData {
    pub rect: RECT,
    pub block_size: i32,
}

#[derive(Clone, Debug)]
pub enum DrawElement {
    Stroke(StrokeData),
    Rect(RectData),
    Arrow(ArrowData),
    Ellipse(EllipseData),
    Text(TextData),
    Mosaic(MosaicData),
}

#[derive(Clone, Debug)]
pub struct OcrBox {
    pub rect: RECT,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct TranslatedBox {
    pub rect: RECT,
    pub original_text: String,
    pub translated_text: String,
}

#[derive(Clone, Debug)]
pub struct ToolbarButton {
    pub rect: RECT,
    pub text: Vec<u16>,
    pub action: Action,
    pub tooltip: String,
    pub is_active: bool,
}

#[derive(Clone, Debug)]
pub struct RecordingState {
    pub start_time: std::time::Instant,
    pub is_paused: bool,
    pub elapsed_secs: u64,
}

pub struct AppState {
    pub analysis_job: Option<super::analysis::AnalysisJob>,
    pub record_job: Option<std::sync::mpsc::Receiver<Result<super::actions::RecordOutcome, String>>>,
    pub text_lines: Vec<super::text_selection::TextLine>,
    pub selected_spans: Vec<super::text_selection::SelectedSpan>,
    pub status_text: String,
    pub last_saved_path: Option<std::path::PathBuf>,
    pub input_window: Option<isize>,
    pub frame_width: u32,
    pub frame_height: u32,
    pub bgra_pixels: Vec<u8>,
    pub dimmed_bgra_pixels: Vec<u8>,
    pub mode: AppMode,
    pub is_dragging: bool,
    pub start_pt: POINT,
    pub current_pt: POINT,
    pub crop_rect: Option<RECT>,
    pub hovered_rect: Option<RECT>,
    pub elements: Vec<DrawElement>,
    pub redo_stack: Vec<DrawElement>,
    pub current_stroke: Option<Vec<POINT>>,
    pub current_color: u32,
    pub current_thickness: i32,
    pub is_eraser: bool,
    pub buttons: Vec<ToolbarButton>,
    pub color_buttons: Vec<ToolbarButton>,
    pub sub_tool_buttons: Vec<ToolbarButton>,
    pub record_buttons: Vec<ToolbarButton>,
    pub hovered_action: Option<Action>,
    pub result_tx: Option<Sender<Option<CroppedResult>>>,
    pub ui_service: Option<ElementRegionService>,
    pub vp: Option<Arc<VisualPipelineRuntime>>,
    
    // OCR & Translation State
    pub is_selecting_text: bool,
    pub text_selection_start: POINT,
    pub text_selection_current: POINT,
    pub ocr_blocks: Option<Vec<OcrBox>>,
    pub translated_lines: Option<Vec<TranslatedBox>>,
    pub loading_text: String,
    
    // Screen Recording State
    pub recording_session: Option<RecordingState>,
    
    // In-place text input buffer
    pub active_text_input: Option<(POINT, String)>,
}

pub const PREDEFINED_COLORS: [u32; 14] = [
    0x00000000, 0x00808080, 0x000000FF, 0x0000FFFF,
    0x0000FF00, 0x00FFFF00, 0x00FF0000, 0x00FF00FF,
    0x00FFFFFF, 0x00C0C0C0, 0x00000080, 0x00008080,
    0x00008000, 0x00800000,
];

pub fn pt_in_rect(rect: &RECT, pt: POINT) -> bool {
    pt.x >= rect.left && pt.x < rect.right && pt.y >= rect.top && pt.y < rect.bottom
}

pub fn to_wstring(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
