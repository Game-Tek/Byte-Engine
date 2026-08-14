use smallvec::SmallVec;
use utils::RGBA;

/// The `RenderTargets` struct tracks sink-scoped render images and attachment access.
pub struct RenderTargets {
	pub(super) images: Vec<(ghi::BaseImageHandle, ghi::Formats)>,
	/// Maps a sink-scoped name to an image index.
	pub(super) by_name: Vec<(usize, String, usize)>,
	/// Maps sink indices to image indices and access policies, making attachments.
	pub(super) by_sink_index: Vec<(usize, (usize, ghi::AccessPolicies))>,
}

impl Default for RenderTargets {
	fn default() -> Self {
		Self::new()
	}
}

impl RenderTargets {
	pub fn new() -> Self {
		Self {
			images: Vec::with_capacity(32),
			by_name: Vec::with_capacity(32),
			by_sink_index: Vec::with_capacity(32),
		}
	}

	pub fn alias(&mut self, sink_id: usize, orig: &str, alias: &str) {
		if let Some(index) = self.get_image_index(orig, sink_id) {
			self.by_name.push((sink_id, alias.to_string(), index));
		}
	}

	/// Inserts a render-target image for a sink and returns its storage index.
	pub fn insert(&mut self, name: String, sink_id: usize, image: ghi::BaseImageHandle, format: ghi::Formats) -> usize {
		if self.get_image_index(&name, sink_id).is_some() {
			panic!(
				"Render target image '{name}' already exists for sink {sink_id}. The most likely cause is that two render pipeline setup paths create the same named target."
			);
		};

		if self.get_attachment_index(&name, sink_id).is_some() {
			panic!(
				"Render target image '{name}' is already registered as an attachment for sink {sink_id}. The most likely cause is that a target was manually added to the attachment list before insertion."
			);
		}

		let index = self.images.len();
		self.images.push((image, format));
		self.by_name.push((sink_id, name, index));
		self.by_sink_index.push((sink_id, (index, ghi::AccessPolicies::WRITE)));

		index
	}

	pub fn read_from(&mut self, name: &str, sink_id: usize) {
		if self.get_attachment_index(name, sink_id).is_some() {
			return;
		}

		let Some(index) = self.get_image_index(name, sink_id) else {
			log::warn!(
				"Render target image '{name}' does not exist for sink {sink_id}; read attachment was not registered. The most likely cause is that a render pass was added before the pipeline that creates this target."
			);
			return;
		};

		self.by_sink_index.push((sink_id, (index, ghi::AccessPolicies::READ)));
	}

	pub fn write_to(&mut self, name: &str, sink_id: usize) {
		if self.get_attachment_index(name, sink_id).is_some() {
			return;
		}

		let Some(index) = self.get_image_index(name, sink_id) else {
			log::warn!(
				"Render target image '{name}' does not exist for sink {sink_id}; write attachment was not registered. The most likely cause is that a render pass was added before the pipeline that creates this target."
			);
			return;
		};

		self.by_sink_index.push((sink_id, (index, ghi::AccessPolicies::WRITE)));
	}

	pub fn get(&self, name: &str, sink_id: usize) -> Option<&(ghi::BaseImageHandle, ghi::Formats)> {
		self.get_image_index(name, sink_id).and_then(|index| self.images.get(index))
	}

	pub fn get_attachment_infos(&self, sink_id: usize) -> SmallVec<[ghi::AttachmentInformation; 8]> {
		let attachments = self
			.by_sink_index
			.iter()
			.filter_map(|(v, (i, ap))| {
				if *v == sink_id {
					let (image, format) = self.images.get(*i)?;
					Some((image, format, ap))
				} else {
					None
				}
			})
			.filter(|(_, _, access)| access.intersects(ghi::AccessPolicies::WRITE))
			.map(|(image, format, access)| {
				let load = access.intersects(ghi::AccessPolicies::READ);
				let store = access.intersects(ghi::AccessPolicies::WRITE);
				let clear_value = if load {
					ghi::ClearValue::None
				} else {
					ghi::ClearValue::Color(RGBA::black())
				};

				ghi::AttachmentInformation::new(*image, ghi::Layouts::RenderTarget, clear_value, load, store)
				// TODO: contionally pass format
			});

		attachments.collect()
	}

	pub fn get_attachment_infos_for_resources(
		&self,
		sink_id: usize,
		resources: &[(String, ghi::AccessPolicies)],
	) -> SmallVec<[ghi::AttachmentInformation; 8]> {
		let mut accesses_by_name = SmallVec::<[(&str, ghi::AccessPolicies); 8]>::new();
		for (name, access) in resources {
			if let Some((_, existing)) = accesses_by_name
				.iter_mut()
				.find(|(existing_name, _)| *existing_name == name.as_str())
			{
				*existing |= *access;
			} else {
				accesses_by_name.push((name.as_str(), *access));
			}
		}

		accesses_by_name
			.into_iter()
			.filter_map(|(name, access)| {
				if !access.intersects(ghi::AccessPolicies::WRITE) {
					return None;
				}

				let (image, _format) = self.get(name, sink_id)?;
				let load = access.intersects(ghi::AccessPolicies::READ);
				let clear_value = if load {
					ghi::ClearValue::None
				} else {
					ghi::ClearValue::Color(RGBA::black())
				};

				Some(ghi::AttachmentInformation::new(
					*image,
					ghi::Layouts::RenderTarget,
					clear_value,
					load,
					true,
				))
			})
			.collect()
	}

	fn get_image(&self, name: &str, sink_id: usize) -> &ghi::BaseImageHandle {
		let index = self.get_attachment_index(name, sink_id).unwrap();
		&self.images.get(index).unwrap().0
	}

	fn get_image_index(&self, name: &str, sink_id: usize) -> Option<usize> {
		self.by_name
			.iter()
			.rev()
			.find(|(sink, n, _)| *sink == sink_id && n == name)
			.map(|(_, _, i)| *i)
	}

	fn get_attachment_index(&self, name: &str, sink_id: usize) -> Option<usize> {
		let image_index = self.get_image_index(name, sink_id)?;

		self.by_sink_index
			.iter()
			.find_map(|(v, (i, _))| if *v == sink_id && *i == image_index { Some(*i) } else { None })
	}

	pub(super) fn get_images_for_sink(&self, index: usize) -> impl Iterator<Item = &ghi::BaseImageHandle> {
		self.by_sink_index.iter().filter_map(move |(v, (i, _))| {
			if *v != index {
				return None;
			}

			self.images.get(*i).map(|(image, _)| image)
		})
	}
}
