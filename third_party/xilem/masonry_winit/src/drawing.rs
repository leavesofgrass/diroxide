// Copyright 2026 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

//! How a window draws, chosen at run time (diroxide.patch).
//!
//! Upstream chooses one renderer when it is built and panics when the
//! machine has no graphics adapter. Here the road is chosen when the first
//! window draws its first frame:
//!
//! - **The graphics card** (Vello on wgpu), when the adapter asked for
//!   before the surface is made is a real one.
//! - **The processor** (Vello's CPU renderer, presented through
//!   softbuffer, which needs no graphics driver at all), when there is no
//!   adapter, when the only one draws on the processor itself (WARP,
//!   llvmpipe, lavapipe, whose compute shaders Vello's GPU renderer
//!   cannot rely on), when the graphics card's surface cannot be made, or
//!   when the application asks for it ([`Drawing::Processor`]). The whole
//!   frame is drawn each time.
//! - **The hybrid renderer on a software adapter** (Vello's hybrid
//!   renderer, simple shaders, on WARP or llvmpipe), when the processor's
//!   presenter cannot start (a display softbuffer does not know).
//!
//! When no road works, the window panics with [`GRAPHICS_FAILURE`] and
//! every reason, as before, for the application to say in words.
//!
//! [`GRAPHICS_FAILURE`]: crate::app::GRAPHICS_FAILURE

use std::sync::OnceLock;

/// How the windows are to draw: set with
/// [`MasonryState::set_drawing`](crate::app::MasonryState::set_drawing)
/// before the first frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Drawing {
    /// The graphics card when there is a real one, else the processor,
    /// else the hybrid renderer on a software adapter.
    #[default]
    Auto,
    /// The processor, else the hybrid renderer on a software adapter.
    Processor,
    /// The hybrid renderer on a software adapter only (to check that road).
    Hybrid,
}

/// The road a window draws by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Road {
    /// Vello on the graphics card.
    Gpu,
    /// Vello's hybrid renderer on a software adapter.
    Hybrid,
    /// Vello's CPU renderer, presented through softbuffer.
    Processor,
}

impl Road {
    /// The road in words, meaning first: "the graphics card", "the
    /// processor", "the hybrid renderer on a software adapter".
    pub fn words(self) -> &'static str {
        match self {
            Self::Gpu => "the graphics card",
            Self::Hybrid => "the hybrid renderer on a software adapter",
            Self::Processor => "the processor",
        }
    }
}

/// The road the first window took, why, and the adapter the probe found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawingReport {
    /// The road taken.
    pub road: Road,
    /// Why, in words: "no graphics adapter was found", "the only graphics
    /// adapter is a software one (Microsoft Basic Render Driver)", "asked
    /// for".
    pub reason: String,
    /// The adapter the probe found before any surface was made, as "name,
    /// backend, kind" ("NVIDIA GeForce RTX 3060, Vulkan, discrete"), if
    /// any.
    pub adapter: Option<String>,
    /// The roads tried before this one, each with why it failed.
    pub failed: Vec<String>,
}

static REPORT: OnceLock<DrawingReport> = OnceLock::new();

/// The frames presented so far, on whatever road.
static PRESENTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The time spent drawing and presenting them, in microseconds.
static PRESENT_MICROS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The first frame that could not be presented, and why.
static FIRST_FAILURE: OnceLock<String> = OnceLock::new();

/// The frames the windows presented, for an application's log, so a
/// machine with no graphics card shows it really drew, and how long.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frames {
    /// How many were presented.
    pub presented: u64,
    /// The time spent drawing and presenting them.
    pub time: std::time::Duration,
    /// Why the first frame that could not be presented failed, if one did.
    pub failure: Option<String>,
}

/// The frames presented so far.
pub fn frames_presented() -> Frames {
    Frames {
        presented: PRESENTED.load(std::sync::atomic::Ordering::Relaxed),
        time: std::time::Duration::from_micros(
            PRESENT_MICROS.load(std::sync::atomic::Ordering::Relaxed),
        ),
        failure: FIRST_FAILURE.get().cloned(),
    }
}

/// Counts a frame drawn and presented since `started`, or keeps why it
/// could not be.
pub(crate) fn presented_since(started: masonry_core::util::Instant, result: Result<(), String>) {
    if result.is_ok() {
        let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        PRESENT_MICROS.fetch_add(micros, std::sync::atomic::Ordering::Relaxed);
    }
    presented(result);
}

pub(crate) fn presented(result: Result<(), String>) {
    match result {
        Ok(()) => {
            PRESENTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Err(err) => {
            tracing::error!("Couldn't present a frame: {err}");
            let _ = FIRST_FAILURE.set(err);
        }
    }
}

/// The road the first window took, once it has drawn its first frame.
pub fn drawing_report() -> Option<DrawingReport> {
    REPORT.get().cloned()
}

pub(crate) fn set_report(report: DrawingReport) {
    tracing::info!(
        road = report.road.words(),
        reason = report.reason.as_str(),
        adapter = report.adapter.as_deref().unwrap_or("none"),
        "drawing road chosen"
    );
    let _ = REPORT.set(report);
}

/// An adapter's kind in a word.
pub(crate) fn kind_word(kind: wgpu::DeviceType) -> &'static str {
    match kind {
        wgpu::DeviceType::DiscreteGpu => "discrete",
        wgpu::DeviceType::IntegratedGpu => "integrated",
        wgpu::DeviceType::VirtualGpu => "virtual",
        wgpu::DeviceType::Cpu => "CPU",
        wgpu::DeviceType::Other => "other",
    }
}

/// An adapter as "name, backend, kind".
pub(crate) fn adapter_words(info: &wgpu::AdapterInfo) -> String {
    format!(
        "{}, {:?}, {}",
        info.name,
        info.backend,
        kind_word(info.device_type)
    )
}

/// The roads to try, in order, each with why: from the probe's adapter
/// (asked for before any surface is made) and what the application asked.
pub(crate) fn plan(drawing: Drawing, probe: Option<&wgpu::AdapterInfo>) -> Vec<(Road, String)> {
    let fallback = |reason: String| {
        vec![
            (Road::Processor, reason.clone()),
            (
                Road::Hybrid,
                format!("{reason}, and the processor's presenter could not start"),
            ),
        ]
    };
    match drawing {
        Drawing::Hybrid => vec![(Road::Hybrid, "asked for".to_owned())],
        Drawing::Processor => fallback("asked for".to_owned()),
        Drawing::Auto => match probe {
            None => fallback("no graphics adapter was found".to_owned()),
            Some(info) if info.device_type == wgpu::DeviceType::Cpu => fallback(format!(
                "the only graphics adapter is a software one ({})",
                info.name
            )),
            Some(info) => {
                let mut roads = vec![(Road::Gpu, format!("the graphics card {}", info.name))];
                roads.extend(fallback(format!(
                    "the graphics card {} could not draw",
                    info.name
                )));
                roads
            }
        },
    }
}

/// The processor's presenter: Vello's CPU renderer draws the whole frame
/// into a buffer of its own, which is copied into softbuffer's pixels and
/// presented.
#[cfg(feature = "software")]
pub(crate) struct CpuPresenter {
    _context: softbuffer::Context<std::sync::Arc<winit::window::Window>>,
    surface: softbuffer::Surface<
        std::sync::Arc<winit::window::Window>,
        std::sync::Arc<winit::window::Window>,
    >,
    renderer: masonry_imaging::vello_cpu::Renderer,
    rgba: Vec<u8>,
}

#[cfg(feature = "software")]
impl std::fmt::Debug for CpuPresenter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpuPresenter").finish_non_exhaustive()
    }
}

#[cfg(feature = "software")]
impl CpuPresenter {
    /// A presenter for `window`, or why it cannot start.
    pub(crate) fn new(window: std::sync::Arc<winit::window::Window>) -> Result<Self, String> {
        let context = softbuffer::Context::new(std::sync::Arc::clone(&window))
            .map_err(|e| format!("the presenter could not start: {e}"))?;
        let surface = softbuffer::Surface::new(&context, window)
            .map_err(|e| format!("the presenter's surface could not be made: {e}"))?;
        let renderer = masonry_imaging::vello_cpu::new_headless_renderer()
            .map_err(|e| format!("the CPU renderer could not start: {e}"))?;
        Ok(Self {
            _context: context,
            surface,
            renderer,
            rgba: Vec::new(),
        })
    }

    /// Draws `frame` and presents it on `window`.
    pub(crate) fn present(
        &mut self,
        frame: masonry_imaging::PreparedFrame<'_>,
        window: &winit::window::Window,
    ) -> Result<(), String> {
        use masonry_core::imaging::render::{ImageBufferFormat, ImageBufferTarget};
        use masonry_imaging::ImageRenderer as _;
        use std::num::NonZeroU32;

        let (width, height) = (frame.width, frame.height);
        let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            return Ok(());
        };
        self.surface
            .resize(w, h)
            .map_err(|e| format!("the presenter could not resize: {e}"))?;
        let row = width as usize * 4;
        self.rgba.resize(row * height as usize, 0);
        let mut frame = frame;
        self.renderer
            .render_source_into(
                &mut frame,
                ImageBufferTarget {
                    data: &mut self.rgba,
                    width,
                    height,
                    bytes_per_row: row,
                    format: ImageBufferFormat::Rgba8Unorm,
                },
            )
            .map_err(|e| format!("the CPU renderer failed: {e:?}"))?;
        let mut buffer = self
            .surface
            .buffer_mut()
            .map_err(|e| format!("the presenter's buffer was refused: {e}"))?;
        for (out, px) in buffer.iter_mut().zip(self.rgba.chunks_exact(4)) {
            *out = (u32::from(px[0]) << 16) | (u32::from(px[1]) << 8) | u32::from(px[2]);
        }
        window.pre_present_notify();
        buffer
            .present()
            .map_err(|e| format!("the presenter could not present: {e}"))
    }
}
