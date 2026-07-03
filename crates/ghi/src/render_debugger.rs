//! # Render Debugger
//!
//! The render debugger module provides facilities to connect to a render debugger and capture frames for analysis and debugging.

#[cfg(not(target_os = "macos"))]
use renderdoc::{RenderDoc, V141};

/// The render debugger allow the application to connect to a render debugger and capture frames for analysis and debugging.
/// It provides an abstraction over different render debugging tools.
/// It supports RenderDoc.
pub struct RenderDebugger {
	#[cfg(not(target_os = "macos"))]
	renderdoc: Option<RenderDoc<V141>>,
}

impl RenderDebugger {
	/// Creates a new render debugger instance.
	/// It will automatically detect any available render debugger and connect to it.
	pub fn new() -> RenderDebugger {
		#[cfg(not(target_os = "macos"))]
		{
			let renderdoc = RenderDoc::new().ok();
			RenderDebugger { renderdoc }
		}

		#[cfg(target_os = "macos")]
		{
			RenderDebugger {}
		}
	}

	/// Starts a frame capture on the render debugger.
	pub fn start_frame_capture(&mut self) {
		#[cfg(not(target_os = "macos"))]
		if let Some(renderdoc) = self.renderdoc.as_mut() {
			#[cfg(target_os = "linux")]
			renderdoc.start_frame_capture(std::ptr::null_mut(), std::ptr::null_mut());
			// #[cfg(windows)]
			// renderdoc.lock().unwrap().start_frame_capture(std::ptr::null_mut(), std::ptr::null_mut());
		}
	}

	/// Ends a frame capture on the render debugger.
	pub fn end_frame_capture(&mut self) {
		#[cfg(not(target_os = "macos"))]
		if let Some(renderdoc) = self.renderdoc.as_mut() {
			#[cfg(target_os = "linux")]
			renderdoc.end_frame_capture(std::ptr::null_mut(), std::ptr::null_mut());
			// #[cfg(windows)]
			// renderdoc.lock().unwrap().end_frame_capture(std::ptr::null_mut(), std::ptr::null_mut());
		}
	}
}
