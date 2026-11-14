//! Staging for the texels the CPU writes into UI images and the glyph atlas while earlier frames may still sample
//! them.

use ghi::frame::Frame as _;

/// The `UiUploadStaging` struct packs the rows of one frame's image copies into the frame's copy of a staging buffer,
/// so the CPU never writes bytes an earlier frame's copies still read.
///
/// Sum each copy's staging bytes from [`region_layout`], start the frame's copies with [`Self::begin`], write them with
/// [`UiUploadWriter::copy`], then record [`Self::take_copies`] with `copy_buffer_to_images` before the frame's draws.
pub(super) struct UiUploadStaging {
	buffer: ghi::BufferHandle<[u8]>,
	capacity: usize,
	copies: Vec<ghi::BufferImageCopyDescriptor>,
}

impl UiUploadStaging {
	/// The staging bytes each frame starts with, enough for a 128 by 128 RGBA8 image.
	const INITIAL_CAPACITY: usize = 64 * 1024;

	/// Creates staging that grows when a frame's copies need more.
	pub(super) fn new(context: &mut impl ghi::context::ContextCreate) -> Self {
		let buffer = context.build_buffer::<[u8]>(
			ghi::buffer::Builder::new(ghi::Uses::TransferSource)
				.name("UI Image Uploads")
				.host_access(ghi::HostAccess::Write)
				.copies(ghi::Copies::PerFrame)
				.length(Self::INITIAL_CAPACITY),
		);
		Self {
			buffer,
			capacity: Self::INITIAL_CAPACITY,
			copies: Vec::new(),
		}
	}

	/// Starts this frame's copies: grows the staging buffer to at least `size` bytes, then maps the frame's copy.
	pub(super) fn begin<'a>(&'a mut self, frame: &'a mut ghi::implementation::Frame, size: usize) -> UiUploadWriter<'a> {
		if size > self.capacity {
			// Doubling keeps resizes rare while UI images and the atlas grow.
			self.capacity = size.max(self.capacity * 2);
			frame.resize_buffer(self.buffer, self.capacity);
		}
		UiUploadWriter {
			bytes: frame.map(self.buffer),
			buffer: self.buffer.into(),
			offset: 0,
			copies: &mut self.copies,
		}
	}

	/// Returns the copies the current frame queued, in the order they were written, and forgets them, so a later
	/// frame never records them again.
	pub(super) fn take_copies<'a>(&mut self, frame_allocator: &'a bumpalo::Bump) -> &'a [ghi::BufferImageCopyDescriptor] {
		let copies = frame_allocator.alloc_slice_copy(&self.copies);
		self.copies.clear();
		copies
	}
}

/// The `UiUploadWriter` struct writes one frame's copies into the staging bytes [`UiUploadStaging::begin`] mapped.
pub(super) struct UiUploadWriter<'a> {
	bytes: &'a mut [u8],
	buffer: ghi::BaseBufferHandle,
	offset: usize,
	copies: &'a mut Vec<ghi::BufferImageCopyDescriptor>,
}

impl UiUploadWriter<'_> {
	/// Stages `region` of `image`, whose rows start `source_bytes_per_row` apart in `source`, and queues its copy.
	pub(super) fn copy(
		&mut self,
		image: ghi::BaseImageHandle,
		format: ghi::Formats,
		region: ghi::image::Region,
		source: &[u8],
		source_bytes_per_row: usize,
	) {
		let layout = region_layout(format, region);
		layout.write_rows(&mut self.bytes[self.offset..], source, source_bytes_per_row);
		self.copies
			.push(layout.copy(self.buffer, self.offset, image, 0).region(region));
		// Every copy spans whole padded rows, so the next one starts at the copy pitch alignment too.
		self.offset += layout.padded_size;
	}
}

/// Returns how `region` of an image in `format` sits in staging. Its padded size is the staging bytes the copy takes.
pub(super) fn region_layout(format: ghi::Formats, region: ghi::image::Region) -> ghi::ImageCopyLayout {
	ghi::ImageCopyLayout::new(format, utils::Extent::rectangle(region.size[0], region.size[1]), 1).expect(
		"UI image copy size overflowed. The most likely cause is a UI image or glyph atlas too large to stage. Use smaller images.",
	)
}
