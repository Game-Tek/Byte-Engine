//! # Render Debugger
//!
//! Connects the engine to a render debugger for frame capture and analysis.

#[cfg(target_os = "linux")]
use renderdoc::{RenderDoc, V141};

/// The `RenderDebugger` struct provides a backend-independent frame-capture boundary.
///
/// The current implementation supports RenderDoc.
pub struct RenderDebugger {
	#[cfg(target_os = "linux")]
	renderdoc: Option<RenderDoc<V141>>,
}

impl RenderDebugger {
	/// Detects an available render debugger and connects to it.
	pub fn new() -> RenderDebugger {
		#[cfg(target_os = "linux")]
		{
			let renderdoc = RenderDoc::new().ok();
			RenderDebugger { renderdoc }
		}

		#[cfg(not(target_os = "linux"))]
		{
			RenderDebugger {}
		}
	}

	/// Starts a frame capture on the render debugger.
	pub fn start_frame_capture(&mut self) {
		#[cfg(target_os = "linux")]
		if let Some(renderdoc) = self.renderdoc.as_mut() {
			renderdoc.start_frame_capture(std::ptr::null_mut(), std::ptr::null_mut());
		}
	}

	/// Ends a frame capture on the render debugger.
	pub fn end_frame_capture(&mut self) {
		#[cfg(target_os = "linux")]
		if let Some(renderdoc) = self.renderdoc.as_mut() {
			renderdoc.end_frame_capture(std::ptr::null_mut(), std::ptr::null_mut());
		}
	}
}
