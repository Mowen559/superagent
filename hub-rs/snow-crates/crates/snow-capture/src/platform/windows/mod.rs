pub(crate) mod com;
pub(crate) mod cursor;
pub(crate) mod d3d11;
pub(crate) mod dirty_rect;
pub(crate) mod display_change;
pub(crate) mod duplication;
pub(crate) mod gdi;
pub(crate) mod gpu_tonemap;
pub(crate) mod monitor;
pub(crate) mod region_pipeline;
pub(crate) mod surface;
pub(crate) mod wgc;

use std::sync::Arc;
use std::time::Duration;

use crate::backend::{AutoBackendPolicy, CaptureBackend, CaptureBackendKind, MonitorCapturer};
use crate::capture_session::CaptureTargetInfo;
use crate::error::{CaptureError, CaptureErrorClass, CaptureResult};
use crate::monitor::MonitorId;
use crate::region::MonitorLayout;
use crate::window::WindowId;

const AUTO_KIND_ERROR: &str = "auto backend selection is handled separately";

pub(crate) struct WindowsBackend {
    resolver: Arc<monitor::MonitorResolver>,
    kind: CaptureBackendKind,
    auto_policy: AutoBackendPolicy,
}

impl WindowsBackend {
    pub(crate) fn with_kind_and_policy(
        kind: CaptureBackendKind,
        auto_policy: AutoBackendPolicy,
    ) -> CaptureResult<Self> {
        let display_cache = display_change::DisplayInfoCache::new()?;
        let resolver = Arc::new(monitor::MonitorResolver::with_display_cache(display_cache));
        Ok(Self {
            resolver,
            kind,
            auto_policy,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn with_kind_and_policy_for_screenshot(
        kind: CaptureBackendKind,
        auto_policy: AutoBackendPolicy,
    ) -> CaptureResult<Self> {
        let resolver = Arc::new(monitor::MonitorResolver::new(Duration::from_secs(60)));
        Ok(Self {
            resolver,
            kind,
            auto_policy,
        })
    }

    fn create_by_kind(
        &self,
        kind: CaptureBackendKind,
        monitor: &MonitorId,
    ) -> CaptureResult<Box<dyn MonitorCapturer>> {
        match kind {
            CaptureBackendKind::Auto => Err(auto_kind_error()),
            CaptureBackendKind::DxgiDuplication => Ok(Box::new(
                duplication::WindowsMonitorCapturer::new(monitor, self.resolver.clone())?,
            )),
            CaptureBackendKind::WindowsGraphicsCapture => Ok(Box::new(
                wgc::WindowsMonitorCapturer::new(monitor, self.resolver.clone())?,
            )),
            CaptureBackendKind::Gdi => Ok(Box::new(gdi::WindowsMonitorCapturer::new(
                monitor,
                self.resolver.clone(),
            )?)),
        }
    }

    fn create_window_by_kind(
        &self,
        kind: CaptureBackendKind,
        window: &WindowId,
    ) -> CaptureResult<Box<dyn MonitorCapturer>> {
        match kind {
            CaptureBackendKind::Auto => Err(auto_kind_error()),
            CaptureBackendKind::DxgiDuplication => Ok(Box::new(
                duplication::WindowsDxgiWindowCapturer::new(window, self.resolver.clone())?,
            )),
            CaptureBackendKind::WindowsGraphicsCapture => {
                Ok(Box::new(wgc::WindowsWindowCapturer::new(window)?))
            }
            CaptureBackendKind::Gdi => Ok(Box::new(gdi::WindowsWindowCapturer::new(window)?)),
        }
    }

    fn create_auto_capturer(&self, monitor: &MonitorId) -> CaptureResult<Box<dyn MonitorCapturer>> {
        let mut errors: Vec<(CaptureBackendKind, CaptureError)> = Vec::new();

        for kind in self.auto_policy.normalized_priority() {
            match self.create_by_kind(kind, monitor) {
                Ok(capturer) => return Ok(capturer),
                Err(err) => {
                    if matches!(err, CaptureError::MonitorLost) {
                        return Err(err);
                    }
                    errors.push((kind, err));
                }
            }
        }

        Err(CaptureError::BackendUnavailable(format!(
            "failed to initialize auto backend for {}: {}",
            monitor.name(),
            format_backend_errors(&errors)
        )))
    }

    fn create_auto_window_capturer(
        &self,
        window: &WindowId,
    ) -> CaptureResult<Box<dyn MonitorCapturer>> {
        let mut errors: Vec<(CaptureBackendKind, CaptureError)> = Vec::new();

        for kind in self.auto_policy.normalized_priority() {
            match self.create_window_by_kind(kind, window) {
                Ok(capturer) => return Ok(capturer),
                Err(err) if err.class() == CaptureErrorClass::InvalidInput => return Err(err),
                Err(err) => errors.push((kind, err)),
            }
        }

        Err(CaptureError::BackendUnavailable(format!(
            "failed to initialize auto window backend for {}: {}",
            window.stable_id(),
            format_backend_errors(&errors)
        )))
    }
}

fn auto_kind_error() -> CaptureError {
    CaptureError::InvalidConfig(AUTO_KIND_ERROR.to_string())
}

fn format_backend_errors(errors: &[(CaptureBackendKind, CaptureError)]) -> String {
    errors
        .iter()
        .map(|(kind, error)| format!("{}: {error}", kind.as_str()))
        .collect::<Vec<_>>()
        .join("; ")
}

impl CaptureBackend for WindowsBackend {
    fn enumerate_monitors(&self) -> CaptureResult<Vec<MonitorId>> {
        self.resolver.enumerate_monitors()
    }

    fn primary_monitor(&self) -> CaptureResult<MonitorId> {
        self.resolver.primary_monitor()
    }

    fn monitor_layout(&self) -> CaptureResult<MonitorLayout> {
        let monitors = self.resolver.enumerate_monitors()?;
        MonitorLayout::snapshot_from_monitors(monitors)
    }

    fn inspect_window(&self, window: &WindowId) -> CaptureResult<CaptureTargetInfo> {
        use windows::Win32::Foundation::{HWND, RECT};
        use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

        let hwnd = HWND(window.raw_handle() as *mut std::ffi::c_void);
        let mut rect = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut rect) }.map_err(|err| {
            CaptureError::InvalidTarget(format!("failed to inspect window: {err}"))
        })?;

        let width = (rect.right - rect.left).max(0) as u32;
        let height = (rect.bottom - rect.top).max(0) as u32;
        if width == 0 || height == 0 {
            return Err(CaptureError::InvalidTarget(window.stable_id()));
        }

        Ok(CaptureTargetInfo {
            origin_x: rect.left,
            origin_y: rect.top,
            width,
            height,
        })
    }

    fn display_generation(&self) -> Option<u64> {
        self.resolver.display_generation()
    }

    fn refresh_display_configuration(&self) -> CaptureResult<()> {
        self.resolver.refresh_display_configuration()
    }

    fn create_monitor_capturer(
        &self,
        monitor: &MonitorId,
    ) -> CaptureResult<Box<dyn MonitorCapturer>> {
        match self.kind {
            CaptureBackendKind::Auto => self.create_auto_capturer(monitor),
            other => self.create_by_kind(other, monitor),
        }
    }

    fn create_window_capturer(&self, window: &WindowId) -> CaptureResult<Box<dyn MonitorCapturer>> {
        match self.kind {
            CaptureBackendKind::Auto => self.create_auto_window_capturer(window),
            other => self.create_window_by_kind(other, window),
        }
    }
}
