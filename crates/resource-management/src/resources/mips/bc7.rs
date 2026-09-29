//! Provide the BESL BC7 block encoder that the GPU material mip worker runs after filtering.
//!
//! The kernel encodes one 4x4 block per invocation with the search of the `fast` profile of Intel's ISPC texture
//! compressor. [`crate::resources::mips::gpu::MaterialMipGenerator`] compiles it for the active graphics backend and
//! dispatches it over every level of a material texture's mip chain.

/// The BESL source of the BC7 encoder kernel.
///
/// Bind the level to encode as a `Texture2D` at slot 0 and the output as a `vec4u[]` storage buffer at slot 1. The
/// push constant gives the level size in texels and blocks and the index of the level's first block in the output,
/// so one buffer can hold a whole mip chain. Dispatch one invocation per block.
pub(crate) const BC7_ENCODER: &str = include_str!("bc7.besl");

#[cfg(test)]
pub(crate) mod tests {
	use besl::vm::{Buffer, DescriptorBindings, ExecutableProgram, ExecutionConfig, ResourceSlot, Texture, Value};

	use super::BC7_ENCODER;

	/// Instructions one invocation may execute. Encoding a block with partition ranking takes a few million.
	const INSTRUCTION_LIMIT: usize = 200_000_000;

	/// Encodes an RGBA8 image by running the BC7 kernel once per block in the BESL VM.
	pub(crate) fn encode_in_vm(width: u32, height: u32, rgba: &[u8]) -> Vec<[u8; 16]> {
		let program = besl::compile_to_besl(BC7_ENCODER, None).expect("The BC7 kernel should parse and link");
		let executable = ExecutableProgram::compile(program).expect("The BC7 kernel should compile for the VM");

		let mut texture = Texture::new(width, height).expect("The source texture should have a valid size");
		for (index, pixel) in rgba.as_chunks::<4>().0.iter().enumerate() {
			let coordinate = [index as u32 % width, index as u32 / width];
			texture
				.write(coordinate, pixel.map(|channel| f32::from(channel) / 255.0))
				.expect("The source texel should be inside the texture");
		}

		let (blocks_x, blocks_y) = (width.div_ceil(4), height.div_ceil(4));
		let layout = executable
			.buffer_layout(ResourceSlot::new(1))
			.expect("The kernel should declare its block output at slot 1")
			.clone();
		let mut blocks = Buffer::new_array(layout, (blocks_x * blocks_y) as usize).expect("The block buffer should fit");
		let mut push_constant = Buffer::new(
			executable
				.push_constant_layout()
				.expect("The kernel should declare a push constant")
				.clone(),
		);
		for (name, value) in [
			("width", width),
			("height", height),
			("blocks_x", blocks_x),
			("blocks_y", blocks_y),
			("first_block", 0),
		] {
			push_constant
				.write(name, Value::U32(value))
				.expect("The push constant should declare every level field");
		}

		for block_y in 0..blocks_y {
			for block_x in 0..blocks_x {
				let mut descriptors = DescriptorBindings::new();
				descriptors.bind_texture(ResourceSlot::new(0), &mut texture);
				descriptors.bind_buffer(ResourceSlot::new(1), &mut blocks);
				descriptors.bind_push_constant(&mut push_constant);
				let config = ExecutionConfig::new(INSTRUCTION_LIMIT).with_thread_id([block_x, block_y]);
				executable
					.run_main_with_config(&mut descriptors, &config)
					.expect("The BC7 kernel should encode the block");
			}
		}

		(0..(blocks_x * blocks_y) as usize)
			.map(|index| match blocks.read_array_element(index) {
				Ok(Value::Vec4U(words)) => {
					let mut bytes = [0; 16];
					for (chunk, word) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(words) {
						*chunk = word.to_le_bytes();
					}
					bytes
				}
				other => panic!("Block {index} should be a vec4u, found {other:?}"),
			})
			.collect()
	}

	/// Decodes BC7 blocks into an RGBA8 image of the given size, dropping texels past its edges.
	pub(crate) fn decode_image(width: u32, height: u32, blocks: &[[u8; 16]]) -> Vec<u8> {
		let blocks_x = width.div_ceil(4);
		let mut rgba = vec![0; width as usize * height as usize * 4];
		for (block_index, block) in blocks.iter().enumerate() {
			let (block_x, block_y) = (block_index as u32 % blocks_x, block_index as u32 / blocks_x);
			for (texel, color) in decode_block(block).into_iter().enumerate() {
				let (x, y) = (block_x * 4 + texel as u32 % 4, block_y * 4 + texel as u32 / 4);
				if x < width && y < height {
					let offset = (y as usize * width as usize + x as usize) * 4;
					rgba[offset..offset + 4].copy_from_slice(&color);
				}
			}
		}
		rgba
	}

	/// Returns the peak signal-to-noise ratio of `decoded` against `source` over the first `channels` channels.
	pub(crate) fn psnr(source: &[u8], decoded: &[u8], channels: usize) -> f64 {
		let (mut squared, mut count) = (0.0, 0.0);
		for (source, decoded) in source.as_chunks::<4>().0.iter().zip(decoded.as_chunks::<4>().0) {
			for channel in 0..channels {
				let difference = f64::from(source[channel]) - f64::from(decoded[channel]);
				squared += difference * difference;
				count += 1.0;
			}
		}
		10.0 * (255.0 * 255.0 / (squared / count).max(1e-10)).log10()
	}

	// Test oracle tables from the BC7 format specification, checked against a hardware decoder.
	const PARTITIONS: [u16; 64] = [
		52428, 34952, 61166, 60616, 51328, 65260, 65224, 60544, 51200, 65516, 65152, 59392, 65512, 65280, 65520, 61440, 63248,
		142, 28928, 2254, 140, 29456, 12544, 36046, 2188, 12560, 26214, 13932, 6120, 4080, 29070, 14748, 43690, 61680, 23130,
		13260, 15420, 21930, 38550, 42330, 29646, 5064, 12876, 15324, 27030, 49980, 39270, 1632, 626, 1252, 20032, 10016,
		51510, 37740, 14790, 25500, 37686, 40134, 33150, 59160, 52464, 4044, 30532, 60962,
	];
	const ANCHORS: [u8; 64] = [
		15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 2, 8, 2, 2, 8, 8, 15, 2, 8, 2, 2, 8, 8, 2, 2, 15,
		15, 6, 8, 2, 8, 15, 15, 2, 8, 2, 2, 2, 15, 15, 6, 6, 2, 6, 8, 15, 15, 2, 2, 15, 15, 15, 15, 15, 2, 2, 15,
	];
	const WEIGHTS2: [u32; 4] = [0, 21, 43, 64];
	const WEIGHTS3: [u32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
	const WEIGHTS4: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

	fn weights(bits: u32) -> &'static [u32] {
		match bits {
			2 => &WEIGHTS2,
			3 => &WEIGHTS3,
			4 => &WEIGHTS4,
			_ => unreachable!("BC7 indices have 2, 3, or 4 bits"),
		}
	}

	/// Decodes one BC7 block of the modes the encoder emits: 1, 3, 4, 5, 6, and 7.
	fn decode_block(block: &[u8; 16]) -> [[u8; 4]; 16] {
		let bits = u128::from_le_bytes(*block);
		let mut position = 0;
		let mut read = |count: u32| {
			let value = ((bits >> position) & ((1 << count) - 1)) as u32;
			position += count;
			value
		};
		let mode = bits.trailing_zeros();
		read(mode + 1);

		// (subsets, color bits, alpha bits, p-bit per endpoint, p-bit per subset, index bits, second index bits)
		let (subsets, color_bits, alpha_bits, endpoint_pbits, subset_pbits, index_bits, second_index_bits) = match mode {
			1 => (2, 6, 0, false, true, 3, 0),
			3 => (2, 7, 0, true, false, 2, 0),
			4 => (1, 5, 6, false, false, 2, 3),
			5 => (1, 7, 8, false, false, 2, 2),
			6 => (1, 7, 7, true, false, 4, 0),
			7 => (2, 5, 5, true, false, 2, 0),
			_ => panic!("The encoder should not emit BC7 mode {mode}"),
		};
		let partition = if subsets == 2 { read(6) as usize } else { 0 };
		let rotation = if mode == 4 || mode == 5 { read(2) } else { 0 };
		let index_selection = if mode == 4 { read(1) } else { 0 };

		// Endpoints are listed channel by channel, then p-bits, then indices.
		let endpoint_count = subsets * 2;
		let mut endpoints = [[0_u32; 4]; 4];
		for channel in 0..3 {
			for endpoint in endpoints.iter_mut().take(endpoint_count) {
				endpoint[channel] = read(color_bits);
			}
		}
		for endpoint in endpoints.iter_mut().take(endpoint_count) {
			endpoint[3] = if alpha_bits > 0 { read(alpha_bits) } else { 255 };
		}
		let mut pbits = [0_u32; 4];
		if endpoint_pbits {
			for pbit in pbits.iter_mut().take(endpoint_count) {
				*pbit = read(1);
			}
		} else if subset_pbits {
			for subset in 0..subsets {
				let pbit = read(1);
				pbits[subset * 2] = pbit;
				pbits[subset * 2 + 1] = pbit;
			}
		}
		let has_pbit = endpoint_pbits || subset_pbits;
		let expand = |value: u32, bits: u32| (value << (8 - bits)) | (value >> (2 * bits - 8));
		for (endpoint, pbit) in endpoints.iter_mut().zip(pbits).take(endpoint_count) {
			for channel in 0..4 {
				let bits = if channel == 3 { alpha_bits } else { color_bits };
				if bits == 0 {
					continue;
				}
				let (value, bits) = if has_pbit {
					((endpoint[channel] << 1) | pbit, bits + 1)
				} else {
					(endpoint[channel], bits)
				};
				endpoint[channel] = expand(value, bits);
			}
		}

		let subset_of = |texel: usize| usize::from(subsets == 2 && (PARTITIONS[partition] >> texel) & 1 == 1);
		let is_anchor = |texel: usize| texel == 0 || (subsets == 2 && texel == usize::from(ANCHORS[partition]));
		let indices: [u32; 16] = std::array::from_fn(|texel| read(index_bits - u32::from(is_anchor(texel))));
		let second_indices: [u32; 16] = std::array::from_fn(|texel| {
			if second_index_bits == 0 {
				0
			} else {
				read(second_index_bits - u32::from(texel == 0))
			}
		});

		std::array::from_fn(|texel| {
			let subset = subset_of(texel);
			let (low, high) = (endpoints[subset * 2], endpoints[subset * 2 + 1]);
			let (color_weight, alpha_weight) = if second_index_bits == 0 {
				let weight = weights(index_bits)[indices[texel] as usize];
				(weight, weight)
			} else if index_selection == 0 {
				(
					weights(index_bits)[indices[texel] as usize],
					weights(second_index_bits)[second_indices[texel] as usize],
				)
			} else {
				(
					weights(second_index_bits)[second_indices[texel] as usize],
					weights(index_bits)[indices[texel] as usize],
				)
			};
			let mut color: [u8; 4] = std::array::from_fn(|channel| {
				let weight = if channel == 3 { alpha_weight } else { color_weight };
				(((64 - weight) * low[channel] + weight * high[channel] + 32) >> 6) as u8
			});
			if rotation != 0 {
				color.swap(3, rotation as usize - 1);
			}
			color
		})
	}

	/// Returns a deterministic RGBA8 test image with gradients, hard edges, and noise.
	pub(crate) fn test_image(width: u32, height: u32, transparent: bool) -> Vec<u8> {
		let mut state = 0x2545_f491_u32;
		let mut noise = move || {
			state ^= state << 13;
			state ^= state >> 17;
			state ^= state << 5;
			state % 5
		};
		let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
		for y in 0..height {
			for x in 0..width {
				let edge = x * 3 > y * 2 + width;
				let red = (x * 255 / width.max(1)).min(255);
				let green = if edge { 220 } else { (y * 200 / height.max(1)).min(255) };
				let blue = if (x / 3 + y / 5) % 2 == 0 { 40 } else { 180 };
				let alpha = if transparent { (x * 16 + y * 9) % 256 } else { 255 };
				rgba.extend([red, green, blue].map(|channel| (channel + noise()).min(255) as u8));
				rgba.push(alpha as u8);
			}
		}
		rgba
	}

	/// Encodes an RGBA8 image with the CPU encoder, which uses the same `fast` profile and repeats edge texels into
	/// partial blocks.
	pub(crate) fn encode_on_cpu(width: u32, height: u32, rgba: &[u8]) -> Vec<[u8; 16]> {
		let mut blocks = vec![[0; 16]; (width.div_ceil(4) * height.div_ceil(4)) as usize];
		crate::resources::mips::encode_level_in(
			crate::types::Formats::BC7,
			utils::Extent::rectangle(width, height),
			rgba,
			blocks.as_flattened_mut(),
			std::alloc::Global,
		);
		blocks
	}

	/// Asserts that the kernel encodes `rgba` at least as well as the CPU `fast` profile, within a small tolerance.
	fn assert_matches_cpu_fast_profile(width: u32, height: u32, rgba: &[u8], channels: usize) {
		let kernel = psnr(
			rgba,
			&decode_image(width, height, &encode_in_vm(width, height, rgba)),
			channels,
		);
		let cpu = psnr(
			rgba,
			&decode_image(width, height, &encode_on_cpu(width, height, rgba)),
			channels,
		);

		assert!(
			kernel > cpu - 0.25,
			"the kernel reached {kernel:.2} dB where the CPU fast profile reached {cpu:.2} dB"
		);
	}

	#[test]
	fn uniform_blocks_decode_within_one_step_and_opaque_blocks_stay_opaque() {
		// Odd and even channels need endpoints on both sides of the color when p-bits constrain the stored values.
		for color in [
			[37_u8, 120, 201, 255],
			[0, 255, 128, 255],
			[37, 120, 201, 77],
			[254, 1, 127, 0],
		] {
			let rgba = color.repeat(16);

			let decoded = decode_image(4, 4, &encode_in_vm(4, 4, &rgba));

			for texel in decoded.as_chunks::<4>().0 {
				assert!(
					color
						.iter()
						.zip(texel)
						.all(|(source, decoded)| source.abs_diff(*decoded) <= 1),
					"uniform color {color:?} decoded to {texel:?}"
				);
				if color[3] == 255 {
					assert_eq!(texel[3], 255, "opaque texels must stay exactly opaque");
				}
			}
		}
	}

	#[test]
	fn opaque_image_matches_the_cpu_fast_profile_and_stays_opaque() {
		let (width, height) = (16, 8);
		let rgba = test_image(width, height, false);

		let decoded = decode_image(width, height, &encode_in_vm(width, height, &rgba));

		assert!(decoded.as_chunks::<4>().0.iter().all(|texel| texel[3] == 255));
		assert_matches_cpu_fast_profile(width, height, &rgba, 3);
	}

	#[test]
	fn transparent_image_matches_the_cpu_fast_profile() {
		let (width, height) = (16, 16);
		let rgba = test_image(width, height, true);

		assert_matches_cpu_fast_profile(width, height, &rgba, 4);
	}

	#[test]
	fn partial_edge_blocks_cover_every_texel_of_the_level() {
		let (width, height) = (5, 3);
		let rgba = test_image(width, height, false);

		assert_eq!(encode_in_vm(width, height, &rgba).len(), 2);
		assert_matches_cpu_fast_profile(width, height, &rgba, 3);
	}
}
