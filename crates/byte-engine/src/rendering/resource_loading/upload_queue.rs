use std::collections::VecDeque;

use super::{RenderResource, ResourceLoader, ResourceToken};

/// The `ResourceUploadStore` trait keeps GPU placement and resident identity under renderer control.
///
/// Implement this beside the renderer's buffers, images, allocation tables,
/// and bindless slots. [`FrameUploadQueue::record_frame`] calls [`Self::record`]
/// only after it has claimed the resource revision as uploading. Validate every
/// capacity needed by one upload before recording commands or committing
/// allocation metadata: an error cannot roll back commands already recorded.
///
/// The shared queue owns [`Self::Upload`] until the submitted frame completes,
/// so staging leases and detached upload metadata remain alive for the complete
/// GPU-use interval. After recording, it returns each [`Self::Resident`] with
/// its token; publish that value into scene-visible maps only then.
pub trait ResourceUploadStore {
	/// Prepared value borrowed for recording and retained through GPU completion.
	type Upload;
	/// Renderer-owned handle or metadata published after GPU completion.
	type Resident;
	/// Renderer storage failure reported at transfer recording time.
	type Error;

	/// Records one upload and reserves the renderer-owned value published after GPU completion.
	///
	/// Keep source memory borrowed from `upload`; the queue retains it. The
	/// returned resident may identify storage immediately, but callers must not
	/// make it render-visible until [`FrameUploadQueue::retire_frame`] returns it.
	fn record(
		&mut self,
		recording: &mut ghi::implementation::CommandBufferRecording<'_>,
		upload: &Self::Upload,
	) -> Result<Self::Resident, Self::Error>;
}

/// The `FrameUploadQueue` struct connects prepared values to renderer storage without shortening GPU lifetimes.
///
/// The forward path is [`Self::enqueue`] then [`Self::record_frame`]. The return
/// path is [`Self::retire_frame`] then renderer publication. Keep this queue on
/// the render thread beside its [`ResourceLoader`] and
/// [`ResourceUploadStore`]. It deliberately knows neither the upload layout nor
/// the resident representation.
pub struct FrameUploadQueue<U, Resident> {
	pending: VecDeque<(ResourceToken, U)>,
	submitted: VecDeque<(ghi::FrameKey, Vec<(ResourceToken, U, Resident)>)>,
}

impl<U, Resident> Default for FrameUploadQueue<U, Resident> {
	fn default() -> Self {
		Self {
			pending: VecDeque::new(),
			submitted: VecDeque::new(),
		}
	}
}

impl<U, Resident> FrameUploadQueue<U, Resident> {
	/// Queues one prepared value for the next transfer recording.
	///
	/// Enqueue only successful current completions from
	/// [`ResourceLoader::take_completion`]. The resource remains loading until
	/// [`Self::record_frame`] claims the token.
	pub fn enqueue(&mut self, token: ResourceToken, upload: U) {
		self.pending.push_back((token, upload));
	}

	/// Returns whether at least one current or stale upload is waiting to be examined.
	///
	/// A pipeline manager can return this from
	/// [`crate::rendering::PipelineManager::begin_frame`] to request an upload
	/// command recording. Stale work is filtered during recording.
	pub fn has_pending(&self) -> bool {
		!self.pending.is_empty()
	}

	/// Records every current upload and retains its source data until `frame` completes.
	///
	/// Call this from
	/// [`crate::rendering::PipelineManager::record_frame_uploads`] with the same
	/// frame key submitted for this command buffer. The queue claims each token
	/// before invoking the store, drops stale revisions without touching storage,
	/// and marks store errors failed. Report returned failures before retrying.
	pub fn record_frame<R, S>(
		&mut self,
		frame: ghi::FrameKey,
		recording: &mut ghi::implementation::CommandBufferRecording<'_>,
		loader: &mut ResourceLoader<R>,
		store: &mut S,
	) -> Vec<(ResourceToken, S::Error)>
	where
		R: RenderResource,
		S: ResourceUploadStore<Upload = U, Resident = Resident>,
	{
		let mut submitted = Vec::with_capacity(self.pending.len());
		let mut failed = Vec::new();
		while let Some((token, upload)) = self.pending.pop_front() {
			// Claim the lifecycle transition before the renderer mutates storage or
			// records commands. Duplicate or otherwise stale uploads are discarded.
			if !loader.mark_uploading(token) {
				continue;
			}
			match store.record(recording, &upload) {
				Ok(resident) => submitted.push((token, upload, resident)),
				Err(error) => {
					loader.mark_failed(token);
					failed.push((token, error));
				}
			}
		}
		if !submitted.is_empty() {
			self.submitted.push_back((frame, submitted));
		}
		failed
	}

	/// Returns current residents and drops their upload values after the matching frame completes.
	///
	/// Call this early in [`crate::rendering::PipelineManager::begin_frame`] with
	/// the renderer's completed frame key. Batches for other frame keys remain
	/// retained. Dropping each upload returns any staging lease before the
	/// resident is marked ready and returned for scene publication.
	pub fn retire_frame<R: RenderResource>(
		&mut self,
		completed_frame: Option<ghi::FrameKey>,
		loader: &mut ResourceLoader<R>,
	) -> Vec<(ResourceToken, Resident)> {
		let Some(completed_frame) = completed_frame else {
			return Vec::new();
		};
		let mut completed = Vec::new();
		while self.submitted.front().is_some_and(|(frame, _)| *frame == completed_frame) {
			let (_, uploads) = self
				.submitted
				.pop_front()
				.expect("The completed upload batch was checked before removal.");
			for (token, upload, resident) in uploads {
				// Drop the renderer upload only after the GPU has stopped using its
				// staging lease and detached preparation metadata.
				drop(upload);
				if loader.mark_ready(token) {
					completed.push((token, resident));
				}
			}
		}
		completed
	}
}

#[cfg(test)]
mod tests {
	use std::sync::{
		Arc,
		atomic::{AtomicUsize, Ordering},
	};

	use super::*;

	enum TestResource {}

	impl RenderResource for TestResource {
		type Key = &'static str;
		type Request = ();
		type Prepared = ();
		type Error = ();
	}

	struct DropProbe(Arc<AtomicUsize>);

	impl Drop for DropProbe {
		fn drop(&mut self) {
			self.0.fetch_add(1, Ordering::Relaxed);
		}
	}

	#[test]
	fn upload_lives_until_its_exact_frame_completes() {
		let (mut loader, _endpoint) = ResourceLoader::<TestResource>::new(8, 1);
		let reference = loader.request("mesh", ()).expect("resource registry capacity");
		assert_eq!(loader.submit_requests(1), 1);
		let token = loader.token(reference).expect("requested resource token");
		assert!(loader.mark_uploading(token));
		assert!(!loader.cancel(reference));

		let dropped = Arc::new(AtomicUsize::new(0));
		let frame = ghi::queue::completed_frame_key(2, 2).expect("first reusable frame");
		let other_frame = ghi::queue::completed_frame_key(3, 2).expect("second reusable frame");
		let mut queue = FrameUploadQueue::<DropProbe, usize>::default();
		queue
			.submitted
			.push_back((frame, vec![(token, DropProbe(Arc::clone(&dropped)), 41)]));

		assert!(queue.retire_frame(Some(other_frame), &mut loader).is_empty());
		assert_eq!(dropped.load(Ordering::Relaxed), 0);
		assert_eq!(loader.state(reference), super::super::ResourceState::Uploading);

		let completed = queue.retire_frame(Some(frame), &mut loader);
		assert_eq!(completed.len(), 1);
		assert_eq!(completed.into_iter().next().expect("completed upload"), (token, 41));
		assert_eq!(dropped.load(Ordering::Relaxed), 1);
		assert_eq!(loader.state(reference), super::super::ResourceState::Ready);
	}
}
