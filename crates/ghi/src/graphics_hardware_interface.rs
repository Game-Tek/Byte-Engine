//! Defines backend-independent handles and resource descriptions for GPU rendering.
//!
//! These types do not require a specific render-pipeline architecture.

mod handles;
mod queue;
mod resources;

pub use handles::*;
pub use queue::*;
pub use resources::*;
#[cfg(test)]
pub(super) mod tests {
	use utils::Extent;

	use super::*;
	use crate::{ChannelBitSize, ChannelLayout, Encodings, Formats, Layouts, Size as _, descriptors};

	#[test]
	fn attachment_layer_builders_keep_single_and_layered_rendering_distinct() {
		let single_layer =
			AttachmentInformation::new(BaseImageHandle(1), Layouts::RenderTarget, ClearValue::Depth(0.0), false, true).layer(3);

		assert_eq!(single_layer.layer, Some(3));
		assert_eq!(single_layer.layer_count, None);

		let layered =
			AttachmentInformation::new(BaseImageHandle(1), Layouts::RenderTarget, ClearValue::Depth(0.0), false, true)
				.layers(4);

		assert_eq!(layered.layer, None);
		assert_eq!(layered.layer_count.map(std::num::NonZeroU32::get), Some(4));
		assert_eq!(AttachmentInformation::render_pass_layer_count(&[layered, layered]), 4);
	}

	#[test]
	#[should_panic(expected = "Cannot select one attachment layer after enabling layered rendering")]
	fn attachment_rejects_layer_after_one_layer_layered_rendering() {
		AttachmentInformation::new(BaseImageHandle(1), Layouts::RenderTarget, ClearValue::Depth(0.0), false, true)
			.layers(1)
			.layer(0);
	}

	#[test]
	#[should_panic(expected = "Layered rendering requires at least one attachment layer")]
	fn attachment_rejects_empty_layered_rendering() {
		AttachmentInformation::new(BaseImageHandle(1), Layouts::RenderTarget, ClearValue::Depth(0.0), false, true).layers(0);
	}

	#[test]
	#[should_panic(expected = "Cannot enable layered rendering after selecting one attachment layer")]
	fn attachment_rejects_layered_rendering_after_layer() {
		AttachmentInformation::new(BaseImageHandle(1), Layouts::RenderTarget, ClearValue::Depth(0.0), false, true)
			.layer(0)
			.layers(1);
	}

	#[test]
	#[should_panic(expected = "Render-pass attachments use different layer counts")]
	fn render_pass_rejects_mixed_attachment_layer_counts() {
		let target = BaseImageHandle(1);
		let single = AttachmentInformation::new(target, Layouts::RenderTarget, ClearValue::Depth(0.0), false, true);
		let layered = AttachmentInformation::new(target, Layouts::RenderTarget, ClearValue::Depth(0.0), false, true).layers(4);

		AttachmentInformation::render_pass_layer_count(&[single, layered]);
	}

	#[test]
	fn test_formats_encoding() {
		// Test floating point formats

		assert_eq!(Formats::R8F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::R16F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::R32F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RG8F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RG16F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RGB8F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RGB16F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RGBA8F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RGBA16F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::Depth32.encoding(), Some(Encodings::FloatingPoint));

		// Test unsigned normalized formats

		assert_eq!(Formats::R8UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::R16UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::R32UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RG8UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RG16UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGB8UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGB16UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGBA8UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGBA16UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::Depth16.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGBu11u11u10.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::BGRAu8.encoding(), Some(Encodings::UnsignedNormalized));

		// Test signed normalized formats

		assert_eq!(Formats::R8SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::R16SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::R32SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RG8SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RG16SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RGB8SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RGB16SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RGBA8SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RGBA16SNORM.encoding(), Some(Encodings::SignedNormalized));

		// Test sRGB formats

		assert_eq!(Formats::R8sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::R16sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::R32sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RG8sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RG16sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RGB8sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RGB16sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RGBA8sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RGBA16sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::BGRAsRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::BC7SRGB.encoding(), Some(Encodings::sRGB));

		// Test formats without encoding

		assert_eq!(Formats::U32.encoding(), None);
		assert_eq!(Formats::BC5.encoding(), None);
		assert_eq!(Formats::BC7.encoding(), None);
	}

	#[test]
	fn descriptor_write_constructors_preserve_set_slot_array_and_frame_semantics() {
		let set = DescriptorSetHandle(1);
		let slot = crate::shader::ResourceSlot::new(9);
		let buffer = BaseBufferHandle(2);
		let image = ImageHandle(BaseImageHandle(3));
		let sampler = SamplerHandle(4);
		let acceleration_structure = TopLevelAccelerationStructureHandle(5);

		let buffer_write = descriptors::DescriptorWrite::buffer(set, slot, buffer);

		assert_eq!(buffer_write.descriptor_set, set);
		assert_eq!(buffer_write.slot, slot);
		assert_eq!(buffer_write.array_element, 0);
		assert_eq!(buffer_write.frame_offset, None);
		assert!(matches!(
			buffer_write.descriptor,
			descriptors::WriteData::Buffer {
				handle,
				size: Ranges::Whole
			} if handle == buffer
		));

		let image_write = descriptors::DescriptorWrite::image_with_frame(set, slot, image, Layouts::General, -1);

		assert_eq!(image_write.frame_offset, Some(-1));
		assert!(matches!(
			image_write.descriptor,
			descriptors::WriteData::Image {
				handle,
				layout: Layouts::General,
				mip_level: None,
			} if handle == BaseImageHandle(3)
		));
		let mip_write = descriptors::DescriptorWrite::image_mip(set, slot, image, Layouts::Read, 2);

		assert!(matches!(
			mip_write.descriptor,
			descriptors::WriteData::Image {
				handle,
				layout: Layouts::Read,
				mip_level: Some(2),
			} if handle == BaseImageHandle(3)
		));

		let array_write = descriptors::DescriptorWrite::combined_image_sampler_array_with_frame(
			set,
			slot,
			image,
			sampler,
			Layouts::Read,
			7,
			2,
		);

		assert_eq!(array_write.array_element, 7);
		assert_eq!(array_write.frame_offset, Some(2));
		assert!(matches!(
			array_write.descriptor,
			descriptors::WriteData::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout: Layouts::Read,
				layer: None,
			} if image_handle == BaseImageHandle(3) && sampler_handle == sampler
		));

		let sampler_write = descriptors::DescriptorWrite::sampler(set, slot, sampler);

		assert!(matches!(sampler_write.descriptor, descriptors::WriteData::Sampler(value) if value == sampler));
		let acceleration_write = descriptors::DescriptorWrite::acceleration_structure(set, slot, acceleration_structure);

		assert!(matches!(
			acceleration_write.descriptor,
			descriptors::WriteData::AccelerationStructure { handle } if handle == acceleration_structure
		));
	}

	#[test]
	fn descriptor_write_variants_without_frame_offsets_remain_frame_invariant() {
		let set = DescriptorSetHandle(8);
		let slot = crate::shader::ResourceSlot::new(12);
		let image = ImageHandle(BaseImageHandle(9));
		let sampler = SamplerHandle(10);

		let image_write = descriptors::DescriptorWrite::image(set, slot, image, Layouts::Read);
		let combined = descriptors::DescriptorWrite::combined_image_sampler(set, slot, image, sampler, Layouts::Read);
		let array = descriptors::DescriptorWrite::combined_image_sampler_array(set, slot, image, sampler, Layouts::Read, 3);

		assert_eq!(image_write.frame_offset, None);
		assert_eq!(combined.frame_offset, None);
		assert_eq!(array.frame_offset, None);
		assert_eq!(array.array_element, 3);
	}

	#[test]
	fn test_formats_channel_bit_size() {
		// Test 8-bit formats

		assert_eq!(Formats::R8F.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::R8UNORM.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::R8SNORM.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::R8sRGB.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::RG8F.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::RGB8UNORM.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::RGBA8SNORM.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::BGRAu8.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::BGRAsRGB.channel_bit_size(), ChannelBitSize::Bits8);

		// Test 16-bit formats

		assert_eq!(Formats::R16F.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::R16UNORM.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::RG16SNORM.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::RGB16F.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::RGBA16UNORM.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::Depth16.channel_bit_size(), ChannelBitSize::Bits16);

		// Test 32-bit formats

		assert_eq!(Formats::R32F.channel_bit_size(), ChannelBitSize::Bits32);
		assert_eq!(Formats::R32UNORM.channel_bit_size(), ChannelBitSize::Bits32);
		assert_eq!(Formats::Depth32.channel_bit_size(), ChannelBitSize::Bits32);
		assert_eq!(Formats::U32.channel_bit_size(), ChannelBitSize::Bits32);

		// Test special formats

		assert_eq!(Formats::RGBu11u11u10.channel_bit_size(), ChannelBitSize::Bits11_11_10);
		assert_eq!(Formats::BC5.channel_bit_size(), ChannelBitSize::Compressed);
		assert_eq!(Formats::BC7.channel_bit_size(), ChannelBitSize::Compressed);
		assert_eq!(Formats::BC7SRGB.channel_bit_size(), ChannelBitSize::Compressed);
	}

	#[test]
	fn test_formats_channel_layout() {
		// Test single channel formats

		assert_eq!(Formats::R8F.channel_layout(), ChannelLayout::R);
		assert_eq!(Formats::R16UNORM.channel_layout(), ChannelLayout::R);
		assert_eq!(Formats::R32SNORM.channel_layout(), ChannelLayout::R);
		assert_eq!(Formats::R8sRGB.channel_layout(), ChannelLayout::R);

		// Test dual channel formats

		assert_eq!(Formats::RG8F.channel_layout(), ChannelLayout::RG);
		assert_eq!(Formats::RG16UNORM.channel_layout(), ChannelLayout::RG);
		assert_eq!(Formats::RG8SNORM.channel_layout(), ChannelLayout::RG);

		// Test triple channel formats

		assert_eq!(Formats::RGB8F.channel_layout(), ChannelLayout::RGB);
		assert_eq!(Formats::RGB16UNORM.channel_layout(), ChannelLayout::RGB);
		assert_eq!(Formats::RGB8sRGB.channel_layout(), ChannelLayout::RGB);
		assert_eq!(Formats::RGBu11u11u10.channel_layout(), ChannelLayout::RGB);

		// Test quad channel formats

		assert_eq!(Formats::RGBA8F.channel_layout(), ChannelLayout::RGBA);
		assert_eq!(Formats::RGBA16UNORM.channel_layout(), ChannelLayout::RGBA);
		assert_eq!(Formats::RGBA8SNORM.channel_layout(), ChannelLayout::RGBA);

		// Test BGRA format

		assert_eq!(Formats::BGRAu8.channel_layout(), ChannelLayout::BGRA);
		assert_eq!(Formats::BGRAsRGB.channel_layout(), ChannelLayout::BGRA);

		// Test depth format

		assert_eq!(Formats::Depth16.channel_layout(), ChannelLayout::Depth);
		assert_eq!(Formats::Depth32.channel_layout(), ChannelLayout::Depth);

		// Test packed format

		assert_eq!(Formats::U32.channel_layout(), ChannelLayout::Packed);

		// Test block compressed formats

		assert_eq!(Formats::BC5.channel_layout(), ChannelLayout::BC);
		assert_eq!(Formats::BC7.channel_layout(), ChannelLayout::BC);
		assert_eq!(Formats::BC7SRGB.channel_layout(), ChannelLayout::BC);
	}

	#[test]
	fn test_formats_size() {
		// Test single channel formats

		assert_eq!(Formats::R8F.size(), 1);
		assert_eq!(Formats::R8UNORM.size(), 1);
		assert_eq!(Formats::R16F.size(), 2);
		assert_eq!(Formats::R16UNORM.size(), 2);
		assert_eq!(Formats::R32F.size(), 4);
		assert_eq!(Formats::R32SNORM.size(), 4);

		// Test dual channel formats

		assert_eq!(Formats::RG8F.size(), 2);
		assert_eq!(Formats::RG8UNORM.size(), 2);
		assert_eq!(Formats::RG16F.size(), 4);
		assert_eq!(Formats::RG16SNORM.size(), 4);

		// Test triple channel formats

		assert_eq!(Formats::RGB8F.size(), 3);
		assert_eq!(Formats::RGB8UNORM.size(), 3);
		assert_eq!(Formats::RGB16F.size(), 6);
		assert_eq!(Formats::RGB16SNORM.size(), 6);

		// Test quad channel formats

		assert_eq!(Formats::RGBA8F.size(), 4);
		assert_eq!(Formats::RGBA8UNORM.size(), 4);
		assert_eq!(Formats::RGBA16F.size(), 8);
		assert_eq!(Formats::RGBA16UNORM.size(), 8);

		// Test special formats

		assert_eq!(Formats::RGBu11u11u10.size(), 4);
		assert_eq!(Formats::BGRAu8.size(), 4);
		assert_eq!(Formats::BGRAsRGB.size(), 4);
		assert_eq!(Formats::Depth16.size(), 2);
		assert_eq!(Formats::Depth32.size(), 4);
		assert_eq!(Formats::U32.size(), 4);
		assert_eq!(Formats::BC5.size(), 1);
		assert_eq!(Formats::BC7.size(), 1);
	}

	#[test]
	fn dispatch_extent_rounds_up_partial_groups() {
		let dispatch_extent = DispatchExtent::new(Extent::new(10, 10, 10), Extent::new(5, 5, 5));
		assert_eq!(dispatch_extent.get_extent(), Extent::new(2, 2, 2));

		let dispatch_extent = DispatchExtent::new(Extent::new(10, 10, 10), Extent::new(3, 3, 3));
		assert_eq!(dispatch_extent.get_extent(), Extent::new(4, 4, 4));
	}
}
