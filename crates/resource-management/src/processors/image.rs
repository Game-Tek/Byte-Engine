mod decode;
mod source;

pub(crate) use decode::{decode_rgba16f_in, png_declared_gamma};
pub(crate) use source::canonicalize_rgba16f_in;
pub use source::{ImageSource, SourceChannels, SourceEncoding};
use source::{append_canonical_image_in, canonicalize_image_in, source_bytes_are};

/// The `Semantic` enum tells the image processor how a texture is sampled, so it can pick gamma, format, and packing.
///
/// Importers infer it from material usage, or from the file name through [`guess_semantic_from_name`].
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Semantic {
	Albedo,
	Normal,
	/// A map whose metallic value is read from one channel; stored with all of its channels.
	Metallic,
	/// A map whose roughness value is read from one channel; stored with all of its channels.
	Roughness,
	/// A glTF metallic-roughness map read only through its green and blue channels.
	///
	/// The processor stores it as two channels through [`METALLIC_ROUGHNESS_PACKING`], and material generators remap
	/// their channel reads with the same table. See [`crate::pbr::BrdfMaterialDescription::pack_texture_channels`].
	MetallicRoughness,
	Emissive,
	Height,
	Opacity,
	Displacement,
	AO,
	Other,
}

/// The `ChannelPacking` struct selects which two source channels a packed image keeps, in stored order.
///
/// The image processor moves the selected channels into the first two channels of its filtering surface and stores
/// the result as `RG8` or `RG16`. Material generators remap their channel reads through
/// [`Self::stored_channel`], so a packed image is sampled where it was stored.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct ChannelPacking {
	/// Source channel index stored in each of the two output channels.
	pub source_channels: [usize; 2],
}

impl ChannelPacking {
	/// Returns the stored channel index that holds `source_channel`, or `None` when the packing drops it.
	pub fn stored_channel(self, source_channel: usize) -> Option<usize> {
		self.source_channels.iter().position(|&kept| kept == source_channel)
	}

	/// Moves the selected channels of every RGBA texel to the front, in place.
	///
	/// `SAMPLE` is the byte size of one channel. The remaining channels keep stale values; the level encoder drops
	/// them.
	fn pack_in_place<const SAMPLE: usize>(self, data: &mut [u8]) {
		let [first, second] = self.source_channels;
		for texel in data.as_chunks_mut::<SAMPLE>().0.chunks_exact_mut(4) {
			// Both samples are read before either is written, so any channel pair moves correctly.
			[texel[0], texel[1]] = [texel[first], texel[second]];
		}
	}
}

/// glTF stores roughness in green and metallic in blue; packed maps keep them as red and green.
pub const METALLIC_ROUGHNESS_PACKING: ChannelPacking = ChannelPacking { source_channels: [1, 2] };

/// Returns the packing an image with `semantic` is stored with, or `None` when it keeps all of its channels.
pub fn channel_packing_for_semantic(semantic: Semantic) -> Option<ChannelPacking> {
	(semantic == Semantic::MetallicRoughness).then_some(METALLIC_ROUGHNESS_PACKING)
}

/// The `ImageProcessorSettings` struct lets each caller choose which optional steps the image processor runs for one
/// image, because not every image needs them.
///
/// A sampled material texture wants a mip chain so it doesn't alias when minified, and block compression to save
/// memory; a lookup table read at exact coordinates, such as an IES intensity map, wants neither. Start from
/// [`Self::for_semantic`] or [`Self::LOOKUP`] and pass the result to [`ImageProcessor::process_in`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageProcessorSettings {
	/// Stores a full mip chain down to one texel after the base level. Without it, the image keeps only its base level.
	pub mips: bool,
	/// Block-compresses the stored levels: normal maps to BC5, other images to BC7.
	///
	/// Packed semantics, such as [`Semantic::MetallicRoughness`], and half-float sources are never block-compressed, so
	/// the processor ignores this for them.
	pub compression: bool,
}

impl ImageProcessorSettings {
	/// Settings for tables read at exact coordinates: one uncompressed level, so every stored value is exact.
	pub const LOOKUP: Self = Self {
		mips: false,
		compression: false,
	};

	/// Returns the settings a texture with `semantic` is usually stored with.
	///
	/// Every texture gets a mip chain. Albedo and normal maps are also block-compressed; other maps keep exact values,
	/// because block compression visibly distorts the data channels they hold.
	pub fn for_semantic(semantic: Semantic) -> Self {
		Self {
			mips: true,
			compression: matches!(semantic, Semantic::Albedo | Semantic::Normal),
		}
	}
}

/// The `ImageProcessor` struct is the one image pipeline an asset manager shares between every source format.
///
/// Format handlers only decode pixels into an [`ImageSource`], name its [`Semantic`], and choose its
/// [`ImageProcessorSettings`]; this processor decides what an image resource holds from there: channel packing, the
/// mip chain, block compression, and environment lighting. A PNG, an EXR, or a glTF texture with the same semantic,
/// settings, and source format therefore bakes the same way, and a new source format gets every step without wiring a
/// backend into its handler. Register one with
/// [`crate::asset::manager::AssetManager::set_image_processor`]; handlers reach it through
/// [`crate::asset::handler::BakeContext::image_processor`].
pub struct ImageProcessor {
	mips: MipGenerator,
	ibl: IBLGenerator,
}

impl ImageProcessor {
	/// Creates an image processor that filters and compresses mip chains with `mips` and generates environment lighting
	/// with `ibl`.
	///
	/// Next, register it with [`crate::asset::manager::AssetManager::set_image_processor`].
	pub fn new(mips: MipGenerator, ibl: IBLGenerator) -> Self {
		Self { mips, ibl }
	}

	/// Returns the generator that bakes environment lighting from a decoded linear `RGBA16F` image.
	pub fn ibl(&self) -> &IBLGenerator {
		&self.ibl
	}

	/// Processes decoded pixels into a stored image, using `allocator` for transient and output buffers.
	///
	/// `semantic` selects the stored format and channel packing, `gamma` is the transfer function of the source's color
	/// channels, and `settings` selects the mip chain and block compression. The call suspends while a GPU mip generator
	/// serves the request. Name the result with [`ProcessedImage::into_asset`] and store it.
	///
	/// A decoder that owns its pixel buffer should call [`Self::process_owned_in`] instead, which can store the image
	/// in that buffer without copying it.
	pub async fn process_in<A: Allocator + Clone>(
		&self,
		semantic: Semantic,
		gamma: Gamma,
		settings: ImageProcessorSettings,
		source: ImageSource<'_>,
		allocator: A,
	) -> Result<ProcessedImage<A>, LoadErrors> {
		let layout = SourceLayout::of(source);
		self.process(
			semantic,
			gamma,
			settings,
			layout,
			SourceBytes::Borrowed(source.data),
			allocator,
		)
		.await
	}

	/// Processes decoded pixels the caller hands over, storing the image in their buffer when it can.
	///
	/// When the source's bytes already are the stored base level, the processor keeps `source.data` as the stored
	/// image and filters the mip chain into it after the base level, so the pixels are never copied. Decode into a
	/// buffer with [`Self::in_place_capacity`] bytes of capacity so that also never reallocates. Any other source is
	/// processed as [`Self::process_in`] would, reading the buffer, and allocating from `allocator`.
	pub async fn process_owned_in<A: Allocator + Clone>(
		&self,
		semantic: Semantic,
		gamma: Gamma,
		settings: ImageProcessorSettings,
		source: OwnedImageSource<A>,
		allocator: A,
	) -> Result<ProcessedImage<A>, LoadErrors> {
		let layout = SourceLayout::of(source.as_source());
		self.process(semantic, gamma, settings, layout, SourceBytes::Owned(source.data), allocator)
			.await
	}

	/// Returns the capacity a decoder should reserve for an owned source so [`Self::process_owned_in`] stores the
	/// image in it without copying or reallocating, or `None` when the image won't be stored in the source buffer.
	///
	/// The image is stored in place when its source bytes already are the stored format and it is neither packed nor
	/// block-compressed; the capacity then covers the whole stored chain.
	pub fn in_place_capacity(
		semantic: Semantic,
		gamma: Gamma,
		settings: ImageProcessorSettings,
		extent: Extent,
		channels: SourceChannels,
		encoding: SourceEncoding,
	) -> Option<usize> {
		let layout = SourceLayout {
			extent,
			channels,
			encoding,
		};
		let output_format = layout.output_format(semantic, gamma, settings.compression)?;
		layout
			.stores_in_place(semantic, output_format)
			.then(|| stored_levels(output_format, extent, settings.mips).map(|(_, size)| size))
			.flatten()
	}

	/// Processes one source after the entry points have described it.
	///
	/// Uncompressed, unpacked images store their base level as converted from the source, or as the owned source
	/// itself, and filter every lower level from the one before it inside the stored buffer. Packed and
	/// block-compressed images filter a canonical copy of the source and encode each level into the stored buffer.
	async fn process<A: Allocator + Clone>(
		&self,
		semantic: Semantic,
		gamma: Gamma,
		settings: ImageProcessorSettings,
		layout: SourceLayout,
		bytes: SourceBytes<'_, A>,
		allocator: A,
	) -> Result<ProcessedImage<A>, LoadErrors> {
		let extent = layout.extent;
		let output_format = layout
			.output_format(semantic, gamma, settings.compression)
			.ok_or(LoadErrors::FailedToProcess)?;
		let (streams, size) = stored_levels(output_format, extent, settings.mips).ok_or(LoadErrors::FailedToProcess)?;
		let mips = settings.mips.then_some(&self.mips);

		let data = if layout.stores_in_place(semantic, output_format) {
			let mut data = match bytes {
				// The owned source already is the stored base level, so it becomes the stored image.
				SourceBytes::Owned(data) if data.len() == layout.base_size().ok_or(LoadErrors::FailedToProcess)? => data,
				bytes => {
					let mut data = Vec::with_capacity_in(size, allocator.clone());
					append_canonical_image_in(layout.view(bytes.as_slice()), output_format, &mut data)
						.ok_or(LoadErrors::FailedToProcess)?;
					data
				}
			};
			// Only the lower levels are new bytes; they reuse the reserved capacity after the base level.
			data.resize(size, 0);
			if let Some(mips) = mips {
				mips.filter_mip_chain_in_place(output_format, gamma, extent.width(), extent.height(), &mut data)
					.await
					.map_err(|_| LoadErrors::FailedToProcess)?;
			}
			data
		} else {
			encode_levels_in(
				semantic,
				gamma,
				layout.view(bytes.as_slice()),
				output_format,
				size,
				allocator,
				mips,
			)
			.await?
		};

		let image = Image {
			format: output_format,
			extent: extent.as_array(),
			gamma,
			mip_count: streams.len() as u32,
			ibl: None,
			photometry: None,
		};
		Ok(ProcessedImage { image, streams, data })
	}
}

impl Default for ImageProcessor {
	/// Creates an image processor that runs every step on the CPU, so bakes do not depend on a GPU.
	fn default() -> Self {
		Self::new(MipGenerator::Cpu, IBLGenerator::new())
	}
}

/// The `ProcessedImage` struct holds a processed image before a handler names and stores it.
///
/// Handlers that know more about the image than its pixels, such as the photometric scale of an IES profile, set
/// those [`Image`] fields before calling [`Self::into_asset`].
pub struct ProcessedImage<A: Allocator> {
	pub image: Image,
	/// One stream per stored level, base level first.
	pub streams: Vec<StreamDescription>,
	/// The stored levels back to back. It's a vector, not a boxed slice, so a buffer reserved larger than the image
	/// isn't shrunk, which would copy it in the bake arena.
	pub data: Vec<u8, A>,
}

impl<A: Allocator> ProcessedImage<A> {
	/// Names the image `id` and returns the asset and payload to pass to a [`crate::asset::handler::BakeContext`]
	/// store call.
	pub fn into_asset(self, id: ResourceId<'_>) -> (ProcessedAsset, Vec<u8, A>) {
		(ProcessedAsset::new(id, self.image).with_streams(self.streams), self.data)
	}
}

/// The `OwnedImageSource` struct hands decoded pixels and their buffer to [`ImageProcessor::process_owned_in`], so the
/// processor can keep the buffer as the stored image instead of copying it.
pub struct OwnedImageSource<A: Allocator> {
	pub extent: Extent,
	pub channels: SourceChannels,
	pub encoding: SourceEncoding,
	pub data: Vec<u8, A>,
}

impl<A: Allocator> OwnedImageSource<A> {
	/// Lends the pixels as a borrowed source view.
	pub fn as_source(&self) -> ImageSource<'_> {
		ImageSource::new(self.extent, self.channels, self.encoding, &self.data)
	}
}

/// The `SourceLayout` struct describes source pixels apart from their bytes, so the processor can plan the stored image
/// before it knows whether it borrows or owns them.
#[derive(Clone, Copy)]
struct SourceLayout {
	extent: Extent,
	channels: SourceChannels,
	encoding: SourceEncoding,
}

impl SourceLayout {
	fn of(source: ImageSource<'_>) -> Self {
		Self {
			extent: source.extent,
			channels: source.channels,
			encoding: source.encoding,
		}
	}

	fn view(self, data: &[u8]) -> ImageSource<'_> {
		ImageSource::new(self.extent, self.channels, self.encoding, data)
	}

	/// Returns the format an image with this layout is stored in, or `None` for layouts the processor can't store.
	fn output_format(self, semantic: Semantic, gamma: Gamma, compress: bool) -> Option<Formats> {
		let source_format = self.view(&[]).natural_format()?;
		Some(determine_image_format(source_format, semantic, gamma, compress))
	}

	/// Returns whether the stored base level holds exactly the source's bytes and is its own filtering surface.
	fn stores_in_place(self, semantic: Semantic, output_format: Formats) -> bool {
		channel_packing_for_semantic(semantic).is_none()
			&& stores_filtering_texels(output_format)
			&& source_bytes_are(self.view(&[]), output_format)
	}

	/// Returns the byte size of the source's pixels.
	fn base_size(self) -> Option<usize> {
		let pixels = (self.extent.width() as usize).checked_mul(self.extent.height() as usize)?;
		pixels
			.checked_mul(self.channels.count())?
			.checked_mul(self.encoding.bytes_per_sample())
	}
}

/// The `SourceBytes` enum holds source pixels either lent by the caller or handed over to become the stored image.
enum SourceBytes<'a, A: Allocator> {
	Borrowed(&'a [u8]),
	Owned(Vec<u8, A>),
}

impl<A: Allocator> SourceBytes<'_, A> {
	fn as_slice(&self) -> &[u8] {
		match self {
			Self::Borrowed(data) => data,
			Self::Owned(data) => data,
		}
	}
}

/// Returns one stream per stored level of an `extent` image in `format`, base level first and without padding, and
/// their total size. Without `mips` only the base level is stored.
fn stored_levels(format: Formats, extent: Extent, mips: bool) -> Option<(Vec<StreamDescription>, usize)> {
	let levels = mip_extents(extent.width(), extent.height()).take(if mips { usize::MAX } else { 1 });
	let mut streams = Vec::new();
	let mut size = 0usize;
	for (index, (width, height)) in levels.enumerate() {
		let level_size = encoded_mip_level_size(format, Extent::rectangle(width, height))?;
		streams.push(StreamDescription::new(format!("mip[{index}]"), level_size, size));
		size = size.checked_add(level_size)?;
	}
	Some((streams, size))
}

/// Encodes a packed or block-compressed image of `size` stored bytes, filtering a full mip chain with `mips` when one
/// is given.
///
/// Packed semantics move their kept channels to the front of the filtering surface first, so every level filters and
/// stores the same texels the unpacked image would. A single level is encoded on the CPU.
async fn encode_levels_in<A: Allocator + Clone>(
	semantic: Semantic,
	gamma: Gamma,
	source: ImageSource<'_>,
	output_format: Formats,
	size: usize,
	allocator: A,
	mips: Option<&MipGenerator>,
) -> Result<Vec<u8, A>, LoadErrors> {
	let extent = source.extent;
	// Filtering, block compression, and packing all read a canonical copy of the source.
	let filtering_format = filtering_format(output_format);
	let mut intermediate =
		canonicalize_image_in(source, filtering_format, allocator.clone()).ok_or(LoadErrors::FailedToProcess)?;
	if let Some(packing) = channel_packing_for_semantic(semantic) {
		let intermediate = intermediate.to_mut(allocator.clone());
		match filtering_format {
			Formats::RGBA8 => packing.pack_in_place::<1>(intermediate),
			Formats::RGBA16 => packing.pack_in_place::<2>(intermediate),
			_ => return Err(LoadErrors::FailedToProcess),
		}
	}
	let intermediate = intermediate.as_slice();

	let mut data = Vec::with_capacity_in(size, allocator.clone());
	data.resize(size, 0);
	if let Some(mips) = mips {
		mips.encode_mip_chain(output_format, gamma, extent.width(), extent.height(), intermediate, &mut data)
			.await
			.map_err(|_| LoadErrors::FailedToProcess)?;
	} else {
		encode_level_in(output_format, extent, intermediate, &mut data, allocator);
	}
	Ok(data)
}

/// Guesses how a standalone texture is sampled from the last words of its file name, such as `brick_Base_Color`.
pub fn guess_semantic_from_name(name: ResourceIdBase) -> Semantic {
	// The first suffix that matches wins.
	const SUFFIXES: [(&[&str], Semantic); 11] = [
		(&["base", "color"], Semantic::Albedo),
		(&["albedo"], Semantic::Albedo),
		(&["diffuse"], Semantic::Albedo),
		(&["normal"], Semantic::Normal),
		(&["metallic"], Semantic::Metallic),
		(&["roughness"], Semantic::Roughness),
		(&["emissive"], Semantic::Emissive),
		(&["height"], Semantic::Height),
		(&["opacity"], Semantic::Opacity),
		(&["displacement"], Semantic::Displacement),
		(&["ao"], Semantic::AO),
	];
	SUFFIXES
		.iter()
		.find(|(sequence, _)| has_suffix_token_sequence(name.as_ref(), sequence))
		.map_or(Semantic::Other, |&(_, semantic)| semantic)
}

fn has_suffix_token_sequence(name: &str, sequence: &[&str]) -> bool {
	let name = std::path::Path::new(name)
		.file_stem()
		.and_then(|stem| stem.to_str())
		.unwrap_or(name);
	let mut tokens = name
		.split(|character: char| !character.is_alphanumeric())
		.filter(|token| !token.is_empty())
		.rev();
	!sequence.is_empty()
		&& sequence
			.iter()
			.rev()
			.all(|expected| tokens.next().is_some_and(|token| token.eq_ignore_ascii_case(expected)))
}

pub fn gamma_from_semantic(semantic: Semantic) -> Gamma {
	match semantic {
		Semantic::Albedo | Semantic::Emissive | Semantic::Other => Gamma::SRGB,
		_ => Gamma::Linear,
	}
}

/// Selects the stored format for a source format, given how the image is sampled and whether it may be compressed.
///
/// Eight- and sixteen-bit integer sources keep their depth. Packed semantics store two channels; otherwise, with
/// `compress`, normal maps compress to BC5 and other images to BC7, and everything else stays RGBA. Half-float sources
/// keep their format.
pub fn determine_image_format(source_format: Formats, semantic: Semantic, gamma: Gamma, compress: bool) -> Formats {
	let sixteen_bit = match source_format {
		Formats::RGB8 | Formats::RGBA8 => false,
		Formats::RGB16 | Formats::RGBA16 => true,
		Formats::R16F => return Formats::R16F,
		Formats::RGBA16F => return Formats::RGBA16F,
		_ => panic!("Unsupported format: {:#?}", source_format),
	};
	let packed = channel_packing_for_semantic(semantic).is_some();
	match semantic {
		_ if packed && sixteen_bit => Formats::RG16,
		_ if packed => Formats::RG8,
		Semantic::Normal if compress => Formats::BC5,
		_ if compress && gamma == Gamma::SRGB => Formats::BC7SRGB,
		_ if compress => Formats::BC7,
		_ if sixteen_bit => Formats::RGBA16,
		_ if gamma == Gamma::SRGB => Formats::RGBA8SRGB,
		_ => Formats::RGBA8,
	}
}

#[cfg(test)]
mod tests {
	use std::alloc::Global;

	use utils::Extent;

	use super::{
		ImageProcessor, ImageProcessorSettings, ImageSource, OwnedImageSource, Semantic, SourceChannels, SourceEncoding,
		gamma_from_semantic, guess_semantic_from_name,
	};
	use crate::{
		asset::ResourceId,
		resources::image::Image,
		types::{Formats, Gamma},
	};

	fn image_source(extent: Extent, format: Formats, data: &[u8]) -> ImageSource<'_> {
		ImageSource::from_format(extent, format, data).expect("test format should be a supported image source")
	}

	#[test]
	fn infers_texture_semantic_and_default_gamma_from_asset_name() {
		let cases = [
			("textures/brick_wall_Base_color.png", Semantic::Albedo, Gamma::SRGB),
			("textures/brick_wall_Diffuse.png", Semantic::Albedo, Gamma::SRGB),
			("textures/brick_wall_Albedo.png", Semantic::Albedo, Gamma::SRGB),
			("textures/brick_wall_Normal.png", Semantic::Normal, Gamma::Linear),
			("textures/brick_wall_Metallic.png", Semantic::Metallic, Gamma::Linear),
			("textures/brick_wall_Roughness.png", Semantic::Roughness, Gamma::Linear),
			("textures/brick_wall_Emissive.png", Semantic::Emissive, Gamma::SRGB),
			("textures/brick_wall_Height.png", Semantic::Height, Gamma::Linear),
			("textures/brick_wall_Opacity.png", Semantic::Opacity, Gamma::Linear),
			("textures/brick_wall_Displacement.png", Semantic::Displacement, Gamma::Linear),
			("textures/brick_wall_AO.png", Semantic::AO, Gamma::Linear),
			("textures/brick_wall_Color.png", Semantic::Other, Gamma::SRGB),
			("textures/diffuse_bomb_icon.png", Semantic::Other, Gamma::SRGB),
			("textures/icon_diffuse.png", Semantic::Albedo, Gamma::SRGB),
			("textures/DiffuseBombIcon.png", Semantic::Other, Gamma::SRGB),
			("textures/NormalityChecker.png", Semantic::Other, Gamma::SRGB),
			("textures/AOGenerator.png", Semantic::Other, Gamma::SRGB),
		];

		for (id, expected_semantic, expected_gamma) in cases {
			let semantic = guess_semantic_from_name(ResourceId::new(id).get_base());

			assert_eq!(
				(semantic, gamma_from_semantic(semantic)),
				(expected_semantic, expected_gamma),
				"asset: {id}"
			);
		}
	}

	/// Processes `source` with the usual settings for `semantic`. See [`process_with`].
	async fn process(semantic: Semantic, gamma: Gamma, source: ImageSource<'_>) -> (Image, Vec<usize>, Vec<u8>) {
		process_with(semantic, gamma, ImageProcessorSettings::for_semantic(semantic), source).await
	}

	/// Processes `source` on the CPU image processor and returns the stored image, its streams' offsets, and its
	/// payload.
	async fn process_with(
		semantic: Semantic,
		gamma: Gamma,
		settings: ImageProcessorSettings,
		source: ImageSource<'_>,
	) -> (Image, Vec<usize>, Vec<u8>) {
		let (asset, data) = ImageProcessor::default()
			.process_in(semantic, gamma, settings, source, Global)
			.await
			.expect("Image processing should succeed")
			.into_asset(ResourceId::new("textures/test.png"));
		assert_eq!(asset.id, "textures/test.png");
		assert_eq!(asset.class, "Image");
		let image: Image = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as an image");
		let offsets = asset.streams.iter().flatten().map(|stream| stream.offset()).collect();
		(image, offsets, data)
	}

	#[crate::r#async::test]
	async fn process_image_expands_rgb8_into_an_rgba8_mip_chain() {
		let source = image_source(Extent::rectangle(2, 1), Formats::RGB8, &[1, 2, 3, 4, 5, 6]);

		let (image, offsets, data) = process(Semantic::Other, Gamma::Linear, source).await;

		assert_eq!(image.format, Formats::RGBA8);
		assert_eq!(image.gamma, Gamma::Linear);
		assert_eq!(image.extent, [2, 1, 0]);
		assert_eq!(image.mip_count, 2);
		assert_eq!(offsets, [0, 8]);
		// The 1x1 level averages both texels, rounding to nearest.
		assert_eq!(&*data, &[1, 2, 3, 0xFF, 4, 5, 6, 0xFF, 3, 4, 5, 0xFF]);
	}

	/// Processes an owned RGBA8 source of `extent`, reserved as [`ImageProcessor::in_place_capacity`] asks, and returns
	/// the stored image, whether its payload is the source buffer, and the payload.
	async fn process_owned(
		semantic: Semantic,
		gamma: Gamma,
		settings: ImageProcessorSettings,
		extent: Extent,
		pixels: &[u8],
	) -> (Image, bool, Vec<u8>) {
		let (channels, encoding) = (SourceChannels::RGBA, SourceEncoding::U8);
		let capacity = ImageProcessor::in_place_capacity(semantic, gamma, settings, extent, channels, encoding).unwrap_or(0);
		let mut data = Vec::with_capacity(capacity.max(pixels.len()));
		data.extend_from_slice(pixels);
		let source_buffer = data.as_ptr();
		let source = OwnedImageSource {
			extent,
			channels,
			encoding,
			data,
		};
		let image = ImageProcessor::default()
			.process_owned_in(semantic, gamma, settings, source, Global)
			.await
			.expect("Image processing should succeed");
		(image.image, image.data.as_ptr() == source_buffer, image.data)
	}

	#[crate::r#async::test]
	async fn owned_sources_in_the_stored_format_become_the_stored_chain() {
		let pixels = (0..4 * 4 * 4).map(|index| index as u8 * 3).collect::<Vec<_>>();
		let extent = Extent::rectangle(4, 4);
		let (borrowed, _, expected) =
			process(Semantic::Other, Gamma::SRGB, image_source(extent, Formats::RGBA8, &pixels)).await;

		let (image, kept_buffer, data) = process_owned(
			Semantic::Other,
			Gamma::SRGB,
			ImageProcessorSettings::for_semantic(Semantic::Other),
			extent,
			&pixels,
		)
		.await;

		assert!(kept_buffer, "the stored chain should be the source buffer");
		assert_eq!((image.format, image.mip_count), (borrowed.format, borrowed.mip_count));
		assert_eq!(data, expected, "storing in place must store what a borrowed source stores");
	}

	#[crate::r#async::test]
	async fn owned_sources_that_need_encoding_are_processed_like_borrowed_ones() {
		let pixels = [128_u8; 4 * 4 * 4];
		let extent = Extent::rectangle(4, 4);
		let settings = ImageProcessorSettings::for_semantic(Semantic::Albedo);
		let (_, _, expected) = process(Semantic::Albedo, Gamma::SRGB, image_source(extent, Formats::RGBA8, &pixels)).await;

		assert_eq!(
			ImageProcessor::in_place_capacity(
				Semantic::Albedo,
				Gamma::SRGB,
				settings,
				extent,
				SourceChannels::RGBA,
				SourceEncoding::U8
			),
			None,
			"a BC7 image can't be stored in its source buffer"
		);
		let (image, _, data) = process_owned(Semantic::Albedo, Gamma::SRGB, settings, extent, &pixels).await;

		assert_eq!((image.format, image.mip_count), (Formats::BC7SRGB, 3));
		assert_eq!(data, expected);
	}

	#[crate::r#async::test]
	async fn lookup_settings_keep_one_uncompressed_level() {
		let source = image_source(Extent::rectangle(2, 1), Formats::RGB8, &[1, 2, 3, 4, 5, 6]);

		// An albedo would normally get a BC7 mip chain.
		let (image, offsets, data) =
			process_with(Semantic::Albedo, Gamma::Linear, ImageProcessorSettings::LOOKUP, source).await;

		assert_eq!((image.format, image.mip_count), (Formats::RGBA8, 1));
		assert_eq!(offsets, [0]);
		assert_eq!(&*data, &[1, 2, 3, 0xFF, 4, 5, 6, 0xFF]);
	}

	#[crate::r#async::test]
	async fn uncompressed_settings_keep_the_mip_chain_in_rgba() {
		let source = image_source(Extent::rectangle(4, 4), Formats::RGBA8, &[128; 4 * 4 * 4]);
		let settings = ImageProcessorSettings {
			compression: false,
			..ImageProcessorSettings::for_semantic(Semantic::Albedo)
		};

		let (image, _, data) = process_with(Semantic::Albedo, Gamma::SRGB, settings, source).await;

		assert_eq!((image.format, image.mip_count), (Formats::RGBA8SRGB, 3));
		assert_eq!(data.len(), (16 + 4 + 1) * 4);
	}

	#[crate::r#async::test]
	async fn half_float_images_filter_their_mip_chain_in_linear_values() {
		let texels = [[4.0_f32, 0.5, -0.25, 1.0], [16.0, 2.0, 8.0, 1.0]];
		let source = texels
			.as_flattened()
			.iter()
			.flat_map(|&value| exr::prelude::f16::from_f32(value).to_le_bytes())
			.collect::<Vec<_>>();

		let (image, _, data) = process(
			Semantic::Other,
			Gamma::Linear,
			image_source(Extent::rectangle(2, 1), Formats::RGBA16F, &source),
		)
		.await;
		let values = data
			.as_chunks::<2>()
			.0
			.iter()
			.map(|bytes| exr::prelude::f16::from_le_bytes(*bytes).to_f32())
			.collect::<Vec<_>>();

		assert_eq!((image.format, image.mip_count), (Formats::RGBA16F, 2));
		assert_eq!(&values[8..], &[10.0, 1.25, 3.875, 1.0]);
	}

	/// Processes `source` as a metallic-roughness map and as an unpacked metallic map, and returns both payloads.
	async fn packed_and_unpacked(extent: Extent, format: Formats, source: &[u8]) -> (Image, Vec<u8>, Vec<u8>) {
		let source = image_source(extent, format, source);
		let (packed, _, packed_data) = process(Semantic::MetallicRoughness, Gamma::Linear, source).await;
		let (_, _, unpacked_data) = process(Semantic::Metallic, Gamma::Linear, source).await;
		(packed, packed_data, unpacked_data)
	}

	#[crate::r#async::test]
	async fn process_image_packs_metallic_roughness_green_and_blue_into_rg8() {
		let extent = Extent::rectangle(2, 2);
		let source = [10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 0, 100, 110, 120, 128];

		let (image, data, _) = packed_and_unpacked(extent, Formats::RGBA8, &source).await;

		assert_eq!(image.format, Formats::RG8);
		assert_eq!(image.gamma, Gamma::Linear);
		assert_eq!(&data[..8], &[20, 30, 50, 60, 80, 90, 110, 120]);
	}

	#[crate::r#async::test]
	async fn packed_metallic_roughness_mip_chain_matches_the_unpacked_green_and_blue_channels() {
		// Filtering is per channel, so packing before filtering must store exactly the G and B texels the RGBA8 chain
		// stores, on every level.
		let (width, height) = (8_u32, 4_u32);
		let source = (0..width * height)
			.flat_map(|index| [index as u8, (index * 7 % 251) as u8, (index * 13 % 241) as u8, 255])
			.collect::<Vec<_>>();

		let (image, packed, unpacked) = packed_and_unpacked(Extent::rectangle(width, height), Formats::RGBA8, &source).await;

		assert_eq!(image.format, Formats::RG8);
		assert_eq!(image.mip_count, 4);
		assert_eq!(packed.len(), (32 + 8 + 2 + 1) * 2);
		let expected = unpacked
			.as_chunks::<4>()
			.0
			.iter()
			.flat_map(|texel| [texel[1], texel[2]])
			.collect::<Vec<_>>();
		assert_eq!(&*packed, &*expected);
	}

	#[crate::r#async::test]
	async fn process_image_packs_sixteen_bit_metallic_roughness_into_rg16() {
		let extent = Extent::rectangle(2, 1);
		// RGB16 little-endian texels: (1, 2, 3) and (4, 5, 6).
		let source = [1_u16, 2, 3, 4, 5, 6]
			.iter()
			.flat_map(|value| value.to_le_bytes())
			.collect::<Vec<_>>();

		let (image, data, _) = packed_and_unpacked(extent, Formats::RGB16, &source).await;

		assert_eq!(image.format, Formats::RG16);
		assert_eq!(&data[..8], &[2, 0, 3, 0, 5, 0, 6, 0]);
	}

	#[crate::r#async::test]
	async fn process_image_compresses_rgb16_albedo_to_bc7() {
		// Regression: the old code built an RGBA16 intermediate (8 bytes/pixel) but passed it to
		// the BC7 compressor with stride = width * 4 (an RGBA8 stride), halving the effective row
		// width and producing horizontal stripes. The correct path converts RGB16 → RGBA8 first.
		let extent = Extent::rectangle(4, 4);
		// RGB16: 3 channels × 2 bytes = 6 bytes per pixel
		let source = vec![128_u8; 4 * 4 * 6];

		let settings = ImageProcessorSettings {
			mips: false,
			..ImageProcessorSettings::for_semantic(Semantic::Albedo)
		};

		let (image, _, data) = process_with(
			Semantic::Albedo,
			Gamma::Linear,
			settings,
			image_source(extent, Formats::RGB16, &source),
		)
		.await;

		assert_eq!(image.format, Formats::BC7);
		assert_eq!(image.extent, [4, 4, 0]);
		assert_eq!(image.mip_count, 1);
		// 4×4 image → 1×1 block grid → 1 block × 16 bytes
		assert_eq!(data.len(), 16);
	}

	#[crate::r#async::test]
	async fn process_image_with_mipmaps_produces_correct_mip_count_for_bc5_normal_map() {
		// BC5 compresses RGBA8 intermediate in 4×4 blocks.
		let (width, height) = (8_u32, 8_u32);
		// RGBA8: 4 bytes/pixel
		let source = vec![128_u8; (width * height * 4) as usize];

		let (image, _, data) = process(
			Semantic::Normal,
			Gamma::Linear,
			image_source(Extent::rectangle(width, height), Formats::RGBA8, &source),
		)
		.await;

		// 8×8 → 4×4 → 2×2 → 1×1  =  4 levels
		let expected_levels = crate::resources::mips::mip_level_count(width, height).unwrap();

		assert_eq!(image.mip_count, expected_levels);
		assert_eq!(image.format, Formats::BC5);

		// Level 0: 8×8  → padded 8×8  → 2×2 blocks → 2*2*16 =  64 bytes
		// Level 1: 4×4  → padded 4×4  → 1×1 block  → 1*1*16 =  16 bytes
		// Level 2: 2×2  → padded 4×4  → 1×1 block  →          16 bytes
		// Level 3: 1×1  → padded 4×4  → 1×1 block  →          16 bytes
		let expected_bytes = (2 * 2 * 16) + (1 * 1 * 16) + (1 * 1 * 16) + (1 * 1 * 16);

		assert_eq!(data.len(), expected_bytes);
	}
}

use std::alloc::Allocator;

use utils::Extent;

use crate::{
	ProcessedAsset, StreamDescription,
	asset::{ResourceId, handler::LoadErrors, resource_id::ResourceIdBase},
	ibl::IBLGenerator,
	resources::{
		image::Image,
		mips::{MipGenerator, encode_level_in, encoded_mip_level_size, filtering_format, mip_extents, stores_filtering_texels},
	},
	types::{Formats, Gamma},
};
