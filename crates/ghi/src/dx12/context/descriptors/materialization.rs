use super::super::*;

impl Device {
	pub fn create_descriptor_set(&mut self, _name: Option<&str>) -> DescriptorSetHandle {
		let handle = DescriptorSetHandle(self.descriptor_sets.len() as u64);
		let mut previous: Option<DescriptorSetHandle> = None;

		for _ in 0..self.frames {
			let frame_handle = DescriptorSetHandle(self.descriptor_sets.len() as u64);
			self.descriptor_sets.push(DescriptorSet {
				next: None,
				version: 0,
				descriptors: HashMap::default(),
			});

			if let Some(previous) = previous {
				self.descriptor_sets[previous.0 as usize].next = Some(crate::descriptors::DescriptorSetHandle(frame_handle.0));
			}
			previous = Some(frame_handle);
		}

		handle
	}
	/// Returns the immutable shader-visible heaps for one frame-resolved retained set union.
	///
	/// The flat binding model derives native offsets from the pipeline layout, so the first bind creates the heaps.
	/// Later binds reuse them until a retained write changes one of the participating sets.
	pub(crate) fn materialize_descriptor_heaps(
		&mut self,
		layout_handle: PipelineLayoutHandle,
		sets: &[DescriptorSetHandle],
		sequence_index: u8,
	) -> Option<DescriptorMaterialization> {
		let descriptor_sets = sets
			.iter()
			.map(|&root_set_handle| {
				self.descriptor_set_for_sequence(root_set_handle, sequence_index)
					.unwrap_or(root_set_handle)
			})
			.collect::<SmallVec<[_; 8]>>();
		let versions = descriptor_sets
			.iter()
			.map(|set_handle| {
				self.descriptor_sets
					.get(set_handle.0 as usize)
					.map(|set| set.version)
					.unwrap_or(0)
			})
			.collect::<SmallVec<[_; 8]>>();
		let key = DescriptorMaterializationKey {
			layout: layout_handle,
			descriptor_sets,
			sequence_index,
		};

		if let Some(materialization) = self.descriptor_materializations.get(&key) {
			if materialization.versions == versions {
				return Some(materialization.clone());
			}
		}

		let layout = self.pipeline_layouts.get(layout_handle.0 as usize)?.clone();
		let cbv_srv_uav_heap = (layout.cbv_srv_uav_descriptor_count != 0)
			.then(|| {
				self.create_shader_visible_descriptor_heap(
					D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
					layout.cbv_srv_uav_descriptor_count,
				)
			})
			.flatten();
		let sampler_heap = (layout.sampler_descriptor_count != 0)
			.then(|| {
				self.create_shader_visible_descriptor_heap(D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER, layout.sampler_descriptor_count)
			})
			.flatten();

		if layout.cbv_srv_uav_descriptor_count != 0 && cbv_srv_uav_heap.is_none() {
			return None;
		}
		if layout.sampler_descriptor_count != 0 && sampler_heap.is_none() {
			return None;
		}
		if let Some(heap) = cbv_srv_uav_heap.as_ref() {
			self.initialize_descriptor_heap_defaults(&layout, false, heap, 0);
		}
		if let Some(heap) = sampler_heap.as_ref() {
			self.initialize_descriptor_heap_defaults(&layout, true, heap, 0);
		}

		let mut writes = SmallVec::<[(PipelineResource, u32, RetainedDescriptor); 32]>::new();
		for resource in &layout.resources {
			for set_handle in &key.descriptor_sets {
				let Some(descriptors) = self
					.descriptor_sets
					.get(set_handle.0 as usize)
					.and_then(|set| set.descriptors.get(&resource.descriptor.slot()))
				else {
					continue;
				};
				for (&array_element, &descriptor) in descriptors {
					writes.push((*resource, array_element, descriptor));
				}
			}
		}

		for (resource, array_element, descriptor) in writes {
			if let Some(heap) = cbv_srv_uav_heap.as_ref() {
				self.write_native_descriptor_for_heap(resource, descriptor, array_element, sequence_index, false, heap, 0);
			}
			if let Some(heap) = sampler_heap.as_ref() {
				self.write_native_descriptor_for_heap(resource, descriptor, array_element, sequence_index, true, heap, 0);
			}
		}

		let materialization = DescriptorMaterialization {
			versions,
			cbv_srv_uav_heap,
			sampler_heap,
		};
		self.descriptor_materializations.insert(key, materialization.clone());
		Some(materialization)
	}
	/// Drops cached native snapshots after a resource replacement changes descriptor-visible addresses.
	pub(crate) fn invalidate_descriptor_materializations(&mut self) {
		self.descriptor_materializations.clear();
	}

	/// Drops attachment views whose native resources were replaced.
	pub(crate) fn invalidate_attachment_views_for_resources(&mut self, resources: &[usize]) {
		if resources.is_empty() {
			return;
		}
		self.render_target_views.retain(|key, _| !resources.contains(&key.resource));
		self.depth_stencil_views.retain(|key, _| !resources.contains(&key.resource));
	}

	/// Drops every retained attachment view after swapchain-wide resource replacement.
	pub(crate) fn invalidate_attachment_views(&mut self) {
		self.render_target_views.clear();
		self.depth_stencil_views.clear();
	}
}
