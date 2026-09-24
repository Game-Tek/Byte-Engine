use smallvec::SmallVec;
use utils::Extent;
use utils::RGBA;

/// The `RenderTargets` struct tracks sink-scoped render images and attachment access.
pub struct RenderTargets {
	pub(super) images: Vec<(ghi::BaseImageHandle, ghi::Formats)>,
	/// Divides the sink extent to size each image in `images`, at the same index. One means full sink resolution.
	pub(super) resolution_divisors: Vec<u32>,
	/// Maps a sink-scoped name to an image index.
	pub(super) by_name: Vec<(usize, String, usize)>,
	/// Maps sink indices to image indices and access policies, making attachments.
	pub(super) by_sink_index: Vec<(usize, (usize, ghi::AccessPolicies))>,
	/// Maps a sink-scoped name to a per-frame image that later frames read as history.
	///
	/// History targets are never attachments, so no pass can clear them by accident.
	pub(super) histories: Vec<(usize, String, ghi::DynamicImageHandle, u32)>,
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
			resolution_divisors: Vec::with_capacity(32),
			by_name: Vec::with_capacity(32),
			by_sink_index: Vec::with_capacity(32),
			histories: Vec::with_capacity(8),
		}
	}

	pub fn alias(&mut self, sink_id: usize, orig: &str, alias: &str) {
		if let Some(index) = self.get_image_index(orig, sink_id) {
			self.by_name.push((sink_id, alias.to_string(), index));
		}
	}

	/// Inserts a render-target image for a sink and returns its storage index.
	///
	/// The renderer sizes the image to the sink extent divided by `resolution_divisor`, rounded down and at least one.
	pub fn insert(
		&mut self,
		name: String,
		sink_id: usize,
		image: ghi::BaseImageHandle,
		format: ghi::Formats,
		resolution_divisor: u32,
	) -> usize {
		assert!(
			resolution_divisor > 0,
			"Render target '{name}' has a zero resolution divisor. The most likely cause is a divisor computed from an empty value."
		);
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
		self.resolution_divisors.push(resolution_divisor);
		self.by_name.push((sink_id, name, index));
		self.by_sink_index.push((sink_id, (index, ghi::AccessPolicies::WRITE)));

		index
	}

	/// Registers a per-frame history image for a sink, sized like [`Self::insert`] sizes images.
	pub fn insert_history(&mut self, name: String, sink_id: usize, image: ghi::DynamicImageHandle, resolution_divisor: u32) {
		assert!(
			resolution_divisor > 0,
			"History target '{name}' has a zero resolution divisor. The most likely cause is a divisor computed from an empty value."
		);
		if self.history(&name, sink_id).is_some() || self.get_image_index(&name, sink_id).is_some() {
			panic!(
				"Render target image '{name}' already exists for sink {sink_id}. The most likely cause is that two render pipeline setup paths create the same named target."
			);
		}

		self.histories.push((sink_id, name, image, resolution_divisor));
	}

	/// Returns the per-frame history image registered under `name` for a sink.
	pub fn history(&self, name: &str, sink_id: usize) -> Option<ghi::DynamicImageHandle> {
		self.histories
			.iter()
			.find(|(sink, history_name, _, _)| *sink == sink_id && history_name == name)
			.map(|(_, _, image, _)| *image)
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

	/// Resolves attachments at pass registration, before later passes can rebind names.
	pub fn get_attachment_infos_for_resources(
		&self,
		sink_id: usize,
		resources: &[(&str, ghi::AccessPolicies)],
	) -> SmallVec<[ghi::AttachmentInformation; 8]> {
		let mut accesses_by_name = SmallVec::<[(&str, ghi::AccessPolicies); 8]>::new();
		for (name, access) in resources {
			if let Some((_, existing)) = accesses_by_name.iter_mut().find(|(existing_name, _)| *existing_name == *name) {
				*existing |= *access;
			} else {
				accesses_by_name.push((*name, *access));
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

	pub(crate) fn image(&self, index: usize) -> Option<(ghi::BaseImageHandle, ghi::Formats)> {
		self.images.get(index).copied()
	}

	pub(crate) fn get_image_index(&self, name: &str, sink_id: usize) -> Option<usize> {
		self.by_name
			.iter()
			.rev()
			.find(|(sink, n, _)| *sink == sink_id && n == name)
			.map(|(_, _, i)| *i)
	}

	/// Snapshots current names that resolve to one of the selected image indices.
	pub(crate) fn names_for_images(&self, sink_id: usize, indices: &[usize]) -> Vec<(String, ghi::BaseImageHandle)> {
		self.by_name
			.iter()
			.enumerate()
			.filter(|(position, (sink, _, index))| {
				*sink == sink_id && indices.contains(index) && self.is_current_name_mapping(*position)
			})
			.filter_map(|(_, (_, name, index))| self.images.get(*index).map(|(image, _)| (name.clone(), *image)))
			.collect()
	}

	fn is_current_name_mapping(&self, position: usize) -> bool {
		let (sink, name, _) = &self.by_name[position];
		!self.by_name[position + 1..]
			.iter()
			.any(|(later_sink, later_name, _)| later_sink == sink && later_name == name)
	}

	#[cfg(test)]
	fn name_indices_for_images(&self, sink_id: usize, indices: &[usize]) -> Vec<(String, usize)> {
		self.by_name
			.iter()
			.enumerate()
			.filter(|(position, (sink, _, index))| {
				*sink == sink_id && indices.contains(index) && self.is_current_name_mapping(*position)
			})
			.map(|(_, (_, name, index))| (name.clone(), *index))
			.collect()
	}

	fn get_attachment_index(&self, name: &str, sink_id: usize) -> Option<usize> {
		let image_index = self.get_image_index(name, sink_id)?;

		self.by_sink_index
			.iter()
			.find_map(|(v, (i, _))| if *v == sink_id && *i == image_index { Some(*i) } else { None })
	}

	/// Returns every image, including history images, that follows the sink's extent, with the extent it needs.
	pub(super) fn get_images_for_sink(
		&self,
		index: usize,
		sink_extent: Extent,
	) -> impl Iterator<Item = (ghi::BaseImageHandle, Extent)> {
		let targets = self.by_sink_index.iter().filter_map(move |(v, (i, _))| {
			if *v != index {
				return None;
			}

			let (image, _) = self.images.get(*i)?;
			Some((*image, scaled_extent(sink_extent, self.resolution_divisors[*i])))
		});
		let histories = self
			.histories
			.iter()
			.filter(move |(sink, _, _, _)| *sink == index)
			.map(move |(_, _, image, divisor)| ((*image).into(), scaled_extent(sink_extent, *divisor)));
		targets.chain(histories)
	}
}

/// Divides a sink extent for a reduced-resolution target, keeping every dimension at least one.
fn scaled_extent(extent: Extent, resolution_divisor: u32) -> Extent {
	Extent::rectangle(
		(extent.width() / resolution_divisor).max(1),
		(extent.height() / resolution_divisor).max(1),
	)
}

#[cfg(test)]
mod tests {
	use utils::Extent;

	use super::RenderTargets;

	#[test]
	fn history_targets_follow_the_sink_extent_without_becoming_attachments() {
		let history = ghi::debug::Device::new()
			.build_dynamic_image(ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Image).name("History"));
		let mut targets = RenderTargets::new();

		targets.insert_history("History".into(), 0, history, 2);

		assert_eq!(targets.history("History", 0), Some(history));
		assert_eq!(targets.history("History", 1), None);
		assert_eq!(
			targets.get_images_for_sink(0, Extent::rectangle(1919, 1080)).collect::<Vec<_>>(),
			[(history.into(), Extent::rectangle(959, 540))]
		);
		assert!(targets.get_attachment_infos(0).is_empty());
	}

	#[test]
	fn writable_snapshot_excludes_read_only_images() {
		let mut targets = RenderTargets::new();
		targets.by_name = vec![(0, "written".into(), 3), (0, "read-only".into(), 4)];

		assert_eq!(targets.name_indices_for_images(0, &[3]), [("written".into(), 3)]);
	}

	#[test]
	fn alias_snapshot_is_unchanged_by_a_later_alias() {
		let mut targets = RenderTargets::new();
		targets.by_name = vec![(0, "Bloom Output".into(), 3), (0, "main".into(), 3)];
		let bloom = targets.name_indices_for_images(0, &[3]);

		targets.by_name.push((0, "main".into(), 4));

		assert_eq!(bloom, [("Bloom Output".into(), 3), ("main".into(), 3)]);
		assert_eq!(targets.get_image_index("main", 0), Some(4));
	}

	#[test]
	fn current_snapshot_excludes_an_overwritten_alias() {
		let mut targets = RenderTargets::new();
		targets.by_name = vec![(0, "image-a".into(), 3), (0, "main".into(), 3), (0, "main".into(), 4)];

		assert_eq!(targets.name_indices_for_images(0, &[3]), [("image-a".into(), 3)]);
	}
}
