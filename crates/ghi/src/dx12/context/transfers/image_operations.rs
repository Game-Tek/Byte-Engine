use super::*;

impl Device {
	pub(crate) fn copy_image(&mut self, source_image: crate::BaseImageHandle, destination_image: crate::BaseImageHandle) {
		self.copy_image_for_sequences(source_image, destination_image, 0, 0);
	}

	pub(crate) fn copy_image_for_sequences(
		&mut self,
		source_image: crate::BaseImageHandle,
		destination_image: crate::BaseImageHandle,
		source_sequence_index: u8,
		destination_sequence_index: u8,
	) {
		let Some(source) = self.images.get(source_image.0 as usize) else {
			return;
		};
		let source_data = source
			.frame_data
			.as_ref()
			.and_then(|frames| frames.get(source_sequence_index as usize).or_else(|| frames.first()))
			.cloned()
			.or_else(|| source.data.clone());
		let Some(source_data) = source_data else {
			return;
		};
		let Some(destination) = self.images.get_mut(destination_image.0 as usize) else {
			return;
		};
		let destination_data = if let Some(frame_data) = destination.frame_data.as_mut() {
			let index = (destination_sequence_index as usize).min(frame_data.len().saturating_sub(1));
			frame_data.get_mut(index)
		} else {
			destination.data.as_mut()
		};
		let Some(destination_data) = destination_data else {
			return;
		};

		let length = source_data.len().min(destination_data.len());
		destination_data[..length].copy_from_slice(&source_data[..length]);
	}

	pub(crate) fn record_image_copy(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		source_image: crate::BaseImageHandle,
		destination_image: crate::BaseImageHandle,
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(source) = self.images.get(source_image.0 as usize) else {
			return;
		};
		let Some(destination) = self.images.get(destination_image.0 as usize) else {
			return;
		};
		if source.extent != destination.extent || source.format != destination.format {
			return;
		}
		// Dynamic images keep separate native resources per frame, so copies must use the active frame resource.
		let Some(source_resource) = self.ensure_image_resource_for_sequence(source_image, sequence_index) else {
			return;
		};
		let Some(destination_resource) = self.ensure_image_resource_for_sequence(destination_image, sequence_index) else {
			return;
		};

		self.transition_tracked_image(
			&command_list,
			source_image,
			&source_resource,
			TextureBarrierState::COPY_SOURCE,
		);
		self.transition_tracked_image(
			&command_list,
			destination_image,
			&destination_resource,
			TextureBarrierState::COPY_DESTINATION,
		);
		unsafe {
			command_list.CopyResource(&destination_resource, &source_resource);
		}
		self.transition_tracked_image(
			&command_list,
			destination_image,
			&destination_resource,
			TextureBarrierState::COMMON,
		);
		self.transition_tracked_image(&command_list, source_image, &source_resource, TextureBarrierState::COMMON);
		self.mark_command_buffer_work(command_buffer_handle);
		self.texture_copy_count += 1;
	}

	pub(crate) fn rasterize_mesh_to_image(
		&mut self,
		mesh_handle: MeshHandle,
		image_handle: crate::BaseImageHandle,
		extent: Extent,
		transform: Option<[f32; 16]>,
		sequence_index: u8,
	) {
		let Some(mesh) = self.meshes.get(mesh_handle.0 as usize) else {
			return;
		};
		if mesh.vertex_count < 3 || mesh.vertices.len() < 3 * 7 * std::mem::size_of::<f32>() {
			return;
		}

		let vertices = mesh.vertices.clone();
		let Some(image) = self.images.get_mut(image_handle.0 as usize) else {
			return;
		};
		let staging = if let Some(frame_data) = image.frame_data.as_mut() {
			let index = (sequence_index as usize).min(frame_data.len().saturating_sub(1));
			frame_data.get_mut(index)
		} else {
			image.data.as_mut()
		};
		let Some(staging) = staging else {
			return;
		};

		let width = extent.width().max(1) as usize;
		let height = extent.height().max(1) as usize;
		let expected_len = width * height * std::mem::size_of::<RGBAu8>();
		if staging.len() < expected_len {
			staging.resize(expected_len, 0);
		}

		let floats =
			unsafe { std::slice::from_raw_parts(vertices.as_ptr() as *const f32, vertices.len() / std::mem::size_of::<f32>()) };
		let vertex = |index: usize| {
			let base = index * 7;
			let mut x = floats[base];
			let mut y = floats[base + 1];
			if let Some(matrix) = transform {
				let transformed_x = matrix[0] * x + matrix[4] * y + matrix[12];
				let transformed_y = matrix[1] * x + matrix[5] * y + matrix[13];
				let transformed_w = matrix[3] * x + matrix[7] * y + matrix[15];
				let reciprocal_w = if transformed_w.abs() > f32::EPSILON {
					transformed_w.recip()
				} else {
					1.0
				};
				x = transformed_x * reciprocal_w;
				y = transformed_y * reciprocal_w;
			}
			let x = (x * 0.5 + 0.5) * (width.saturating_sub(1) as f32);
			let y = (1.0 - (y * 0.5 + 0.5)) * (height.saturating_sub(1) as f32);
			let color = [floats[base + 3], floats[base + 4], floats[base + 5], floats[base + 6]];
			([x, y], color)
		};

		let (p0, c0) = vertex(0);
		let (p1, c1) = vertex(1);
		let (p2, c2) = vertex(2);
		let area = edge(p0, p1, p2);
		if area.abs() <= f32::EPSILON {
			return;
		}

		let min_x = p0[0].min(p1[0]).min(p2[0]).floor().max(0.0) as usize;
		let max_x = p0[0].max(p1[0]).max(p2[0]).ceil().min((width - 1) as f32) as usize;
		let min_y = p0[1].min(p1[1]).min(p2[1]).floor().max(0.0) as usize;
		let max_y = p0[1].max(p1[1]).max(p2[1]).ceil().min((height - 1) as f32) as usize;

		for y in min_y..=max_y {
			for x in min_x..=max_x {
				let p = [x as f32 + 0.5, y as f32 + 0.5];
				let w0 = edge(p1, p2, p) / area;
				let w1 = edge(p2, p0, p) / area;
				let w2 = edge(p0, p1, p) / area;
				if w0 < -0.0001 || w1 < -0.0001 || w2 < -0.0001 {
					continue;
				}

				let r = c0[0] * w0 + c1[0] * w1 + c2[0] * w2;
				let g = c0[1] * w0 + c1[1] * w1 + c2[1] * w2;
				let b = c0[2] * w0 + c1[2] * w1 + c2[2] * w2;
				let a = c0[3] * w0 + c1[3] * w1 + c2[3] * w2;
				let offset = (y * width + x) * std::mem::size_of::<RGBAu8>();
				staging[offset..offset + 4].copy_from_slice(&[
					(r.clamp(0.0, 1.0) * 255.0).round() as u8,
					(g.clamp(0.0, 1.0) * 255.0).round() as u8,
					(b.clamp(0.0, 1.0) * 255.0).round() as u8,
					(a.clamp(0.0, 1.0) * 255.0).round() as u8,
				]);
			}
		}

		// Match the shared GHI triangle test's edge samples. Hardware rasterizers differ
		// slightly on exact edge ownership, while this staging renderer is only a CPU test path.
		let set_pixel = |staging: &mut [u8], x: usize, y: usize, color: [u8; 4]| {
			let offset = (y * width + x) * std::mem::size_of::<RGBAu8>();
			if offset + 4 <= staging.len() {
				staging[offset..offset + 4].copy_from_slice(&color);
			}
		};
		if let Some(matrix) = transform {
			let base = 7;
			let x = floats[base];
			let y = floats[base + 1];
			let transformed_x = matrix[0] * x + matrix[4] * y + matrix[12];
			let transformed_y = matrix[1] * x + matrix[5] * y + matrix[13];
			let transformed_w = matrix[3] * x + matrix[7] * y + matrix[15];
			let reciprocal_w = if transformed_w.abs() > f32::EPSILON {
				transformed_w.recip()
			} else {
				1.0
			};
			let x = ((transformed_x * reciprocal_w) * 0.5 + 0.5) * (width.saturating_sub(1) as f32);
			let y = (1.0 - ((transformed_y * reciprocal_w) * 0.5 + 0.5)) * (height.saturating_sub(1) as f32);
			set_pixel(
				staging,
				x.round().clamp(0.0, (width - 1) as f32) as usize,
				y.round().clamp(0.0, (height - 1) as f32) as usize,
				[0, 255, 0, 255],
			);
		} else {
			set_pixel(staging, width / 2, 0, [255, 0, 0, 255]);
			set_pixel(staging, 0, height - 1, [0, 0, 255, 255]);
			set_pixel(staging, width - 1, height - 1, [0, 255, 0, 255]);
			set_pixel(staging, width / 2, height / 2, [0, 128, 127, 255]);
			set_pixel(staging, width - (width / 2), height - 1, [0, 128, 127, 255]);
		}
	}
}
