//! Small immutable textures the engine ships inside its binary, such as SMAA's lookup tables and the blue noise that
//! screen-space passes draw their per-pixel random numbers from.
//!
//! Upload one with [`create_lookup_texture`] when a pass is built, then bind it with
//! [`crate::rendering::set_layout`]'s `sampled` binder method.

use ghi::context::{Context as _, ContextCreate as _};

/// The side of the square, tiling [`BLUE_NOISE`] texture, in texels.
pub(crate) const BLUE_NOISE_SIDE: u32 = 64;

/// Two independent channels of blue noise, interleaved as RG8 texels in rows from the top left.
///
/// Each channel ranks its 4,096 texels so that neighbors get values far apart, so a pass that takes one random sample
/// per pixel leaves fine grain that a small spatial filter removes instead of blotches. Each value from 0 to 255 appears
/// on exactly 16 texels. The tests below regenerate it with the void-and-cluster method.
///
/// Read it at the pixel modulo [`BLUE_NOISE_SIDE`] and add [`blue_noise_offset`], modulo one, to animate it over frames.
pub(crate) const BLUE_NOISE: &[u8; (BLUE_NOISE_SIDE * BLUE_NOISE_SIDE * 2) as usize] =
	include_bytes!("../../assets/rendering/noise/blue-noise-64.rg8");

/// Returns the offset that animates [`BLUE_NOISE`] in frame `frame_index`, one per channel.
///
/// It is point `frame_index` of Martin Roberts' R2 sequence. Adding the same offset to every texel, modulo one, keeps
/// each frame blue, and walks each texel's values along a low-discrepancy sequence, so a temporal filter converges in
/// few frames. The two channels move by different irrational steps, so they don't move in lockstep.
pub(crate) fn blue_noise_offset(frame_index: u64) -> [f32; 2] {
	// The plastic number, the unique real root of x³ = x + 1.
	const PLASTIC: f64 = 1.324_717_957_244_746;
	let frame = frame_index as f64;
	[(frame / PLASTIC).fract() as f32, (frame / (PLASTIC * PLASTIC)).fract() as f32]
}

/// Creates and uploads one immutable lookup texture.
pub(crate) fn create_lookup_texture(
	context: &mut ghi::implementation::Context,
	name: &str,
	format: ghi::Formats,
	extent: utils::Extent,
	pixels: &[u8],
) -> ghi::ImageHandle {
	let image = context.build_image(
		ghi::image::Builder::new(format, ghi::Uses::Image | ghi::Uses::TransferDestination)
			.name(name)
			.extent(extent)
			.device_accesses(ghi::DeviceAccesses::HostToDevice)
			.use_case(ghi::UseCases::STATIC),
	);
	context.get_texture_slice_mut(image).copy_from_slice(pixels);
	context.sync_texture(image);
	image
}

/// Uploads [`BLUE_NOISE`] as an RG8 texture.
pub(crate) fn create_blue_noise_texture(context: &mut ghi::implementation::Context) -> ghi::ImageHandle {
	create_lookup_texture(
		context,
		"Blue Noise",
		ghi::Formats::RG8UNORM,
		utils::Extent::rectangle(BLUE_NOISE_SIDE, BLUE_NOISE_SIDE),
		BLUE_NOISE,
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	const SIDE: usize = BLUE_NOISE_SIDE as usize;
	const TEXEL_COUNT: usize = SIDE * SIDE;
	/// The standard deviation of the Gaussian that measures how crowded a texel's neighborhood is, in texels.
	/// Ulichney's paper recommends 1.5.
	const SIGMA: f64 = 1.5;
	/// Kernel weights are rounded to integers at this scale, so energies are exact sums and ties break the same way on
	/// every platform, whatever its `exp` rounds to.
	const KERNEL_SCALE: f64 = 65536.0;
	const COMMITTED_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/rendering/noise/blue-noise-64.rg8");

	/// Returns the PCG output permutation of `value` ("Hash Functions for GPU Rendering", Jarzynski and Olano).
	fn hash(value: u32) -> u32 {
		let state = value.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
		let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
		(word >> 22) ^ word
	}

	/// Returns the nonzero taps of the toroidal Gaussian kernel as offsets and integer weights.
	fn kernel() -> Vec<(isize, isize, i64)> {
		let half = SIDE as isize / 2;
		let mut taps = Vec::new();
		for dy in -half..half {
			for dx in -half..half {
				let distance_squared = (dx * dx + dy * dy) as f64;
				let weight = ((-distance_squared / (2.0 * SIGMA * SIGMA)).exp() * KERNEL_SCALE).round() as i64;
				if weight > 0 {
					taps.push((dx, dy, weight));
				}
			}
		}
		taps
	}

	/// Adds (`sign` 1) or removes (`sign` -1) the energy a point at `texel` spreads over its neighborhood.
	fn splat(energy: &mut [i64], taps: &[(isize, isize, i64)], texel: usize, sign: i64) {
		let (x, y) = ((texel % SIDE) as isize, (texel / SIDE) as isize);
		let mask = SIDE as isize - 1;
		for &(dx, dy, weight) in taps {
			energy[(((y + dy) & mask) * SIDE as isize + ((x + dx) & mask)) as usize] += sign * weight;
		}
	}

	/// Returns the point with the most crowded neighborhood; the lowest index wins ties.
	fn tightest_cluster(energy: &[i64], pattern: &[bool]) -> usize {
		(0..TEXEL_COUNT)
			.filter(|&texel| pattern[texel])
			.max_by_key(|&texel| (energy[texel], std::cmp::Reverse(texel)))
			.unwrap()
	}

	/// Returns the empty texel with the emptiest neighborhood; the lowest index wins ties.
	fn largest_void(energy: &[i64], pattern: &[bool]) -> usize {
		(0..TEXEL_COUNT)
			.filter(|&texel| !pattern[texel])
			.min_by_key(|&texel| (energy[texel], texel))
			.unwrap()
	}

	/// Ranks every texel by Ulichney's void-and-cluster method ("The void-and-cluster method for dither array
	/// generation", 1993), seeded by `seed`.
	///
	/// It scatters a tenth of the texels at random, then moves the point in the tightest cluster to the largest void
	/// until that changes nothing. Points removed from that pattern, tightest first, take the ranks below its size, and
	/// texels added to it, largest void first, the ranks above. Past half the texels the empty ones are the minority,
	/// but the emptiest neighborhood of points is also the most crowded neighborhood of empty texels, because every
	/// texel's two energies sum to the kernel's total, so the same step finishes the ranking.
	fn void_and_cluster(seed: u32) -> Vec<u16> {
		let taps = kernel();
		let mut pattern = vec![false; TEXEL_COUNT];
		let mut energy = vec![0i64; TEXEL_COUNT];
		let initial_count = TEXEL_COUNT / 10;
		let mut placed = 0;
		for counter in 0.. {
			if placed == initial_count {
				break;
			}
			let texel = hash(hash(counter) ^ seed) as usize % TEXEL_COUNT;
			if !pattern[texel] {
				pattern[texel] = true;
				splat(&mut energy, &taps, texel, 1);
				placed += 1;
			}
		}
		loop {
			let cluster = tightest_cluster(&energy, &pattern);
			pattern[cluster] = false;
			splat(&mut energy, &taps, cluster, -1);
			let void = largest_void(&energy, &pattern);
			pattern[void] = true;
			splat(&mut energy, &taps, void, 1);
			if void == cluster {
				break;
			}
		}

		let mut ranks = vec![0u16; TEXEL_COUNT];
		let (initial_pattern, initial_energy) = (pattern.clone(), energy.clone());
		for rank in (0..initial_count).rev() {
			let cluster = tightest_cluster(&energy, &pattern);
			pattern[cluster] = false;
			splat(&mut energy, &taps, cluster, -1);
			ranks[cluster] = rank as u16;
		}
		let (mut pattern, mut energy) = (initial_pattern, initial_energy);
		for rank in initial_count..TEXEL_COUNT {
			let void = largest_void(&energy, &pattern);
			pattern[void] = true;
			splat(&mut energy, &taps, void, 1);
			ranks[void] = rank as u16;
		}
		ranks
	}

	/// Generates [`BLUE_NOISE`]: two void-and-cluster rankings with different seeds, each rank scaled to a byte.
	fn generate_blue_noise() -> Vec<u8> {
		let channels = [void_and_cluster(0), void_and_cluster(1)];
		(0..TEXEL_COUNT)
			.flat_map(|texel| {
				channels
					.iter()
					.map(move |ranks| (ranks[texel] as usize * 256 / TEXEL_COUNT) as u8)
			})
			.collect()
	}

	/// Writes the committed texture. Run it after changing the generator with
	/// `cargo nextest run -p byte-engine --run-ignored only write_blue_noise`.
	#[test]
	#[ignore = "writes assets/rendering/noise/blue-noise-64.rg8"]
	fn write_blue_noise() {
		std::fs::write(COMMITTED_PATH, generate_blue_noise()).unwrap();
	}

	#[test]
	fn committed_blue_noise_matches_its_generator() {
		assert!(
			generate_blue_noise() == BLUE_NOISE,
			"The committed blue noise differs from what the generator makes. The generator most likely changed without \
			 the file. Run `cargo nextest run -p byte-engine --run-ignored only write_blue_noise` to rewrite it."
		);
	}

	#[test]
	fn blue_noise_uses_every_value_equally_often() {
		for channel in 0..2 {
			let mut counts = [0u32; 256];
			for texel in 0..TEXEL_COUNT {
				counts[BLUE_NOISE[texel * 2 + channel] as usize] += 1;
			}
			assert!(counts.iter().all(|&count| count == 16), "channel {channel}: {counts:?}");
		}
	}

	/// Blue noise has no low frequencies, so every 8x8 window, wrapping around the edges, averages close to the middle
	/// value. White noise strays about 30 from it somewhere in the texture; this blue noise stays within about 10.
	#[test]
	fn blue_noise_windows_average_to_the_middle() {
		for channel in 0..2 {
			for y in 0..SIDE {
				for x in 0..SIDE {
					let sum: u32 = (0..64)
						.map(|tap| BLUE_NOISE[(((y + tap / 8) % SIDE) * SIDE + (x + tap % 8) % SIDE) * 2 + channel] as u32)
						.sum();
					let deviation = (sum as f64 / 64.0 - 127.5).abs();
					assert!(deviation < 16.0, "channel {channel}, window at ({x}, {y}): {deviation}");
				}
			}
		}
	}
}
