//! Small immutable textures the engine ships inside its binary, such as SMAA's lookup tables and the blue noise that
//! screen-space passes draw their per-pixel random numbers from.
//!
//! Upload one with [`create_lookup_texture`] when a pass is built, then bind it with
//! [`crate::rendering::set_layout`]'s `sampled` binder method.

use ghi::command_buffer::CommandBufferRecording as _;
use ghi::context::{Context as _, ContextCreate as _};

/// The side of each square, tiling slice of [`BLUE_NOISE`], in texels.
pub(crate) const BLUE_NOISE_SIDE: u32 = 64;
/// The slices of [`BLUE_NOISE`], one per frame of its cycle.
pub(crate) const BLUE_NOISE_SLICES: u32 = 32;

/// The engine's spatiotemporal blue noise: two independent channels, interleaved as RG8 texels, in
/// [`BLUE_NOISE_SLICES`] slices of [`BLUE_NOISE_SIDE`] rows each, stacked from the top.
///
/// Each slice is blue noise across the image, so a pass that takes one random sample per pixel leaves fine grain that a
/// small spatial filter removes instead of blotches. Each texel's values through the slices are blue noise in time, so
/// a temporal blend of the frames converges faster than with values that change at random. Each value from 0 to 255
/// appears on exactly 512 texels. The tests below regenerate it with Wolfe et al.'s spatiotemporal void-and-cluster
/// method.
///
/// Read it at the pixel modulo [`BLUE_NOISE_SIDE`], in the slice [`blue_noise_slice`] picks for the frame.
pub(crate) const BLUE_NOISE: &[u8; (BLUE_NOISE_SIDE * BLUE_NOISE_SIDE * BLUE_NOISE_SLICES * 2) as usize] =
	include_bytes!("../../assets/rendering/noise/blue-noise-64x64x32.rg8");

/// Returns the slice of [`BLUE_NOISE`] a pass of `sink` reads in frame `frame_index`.
///
/// While the sink is jittered, temporal antialiasing blends the frames, so the noise walks through the slices and the
/// blend averages it out. Otherwise every frame reads slice 0, so the grain holds still instead of crawling. A pass that
/// blends its own frames, such as screen-space GI, reads slice `frame_index` modulo [`BLUE_NOISE_SLICES`] instead.
pub(crate) fn blue_noise_slice(sink: &crate::rendering::Sink, frame_index: u64) -> u32 {
	if sink.is_jittered() {
		(frame_index % u64::from(BLUE_NOISE_SLICES)) as u32
	} else {
		0
	}
}

/// Creates an immutable image from `builder` and fills its base level with `pixels`.
///
/// `pixels` holds every depth slice of every array layer, one after another, each in rows of the image's width without
/// padding. They reach the image through a staging buffer and a copy recorded on `queue`, and the call returns once the
/// GPU wrote them.
pub(crate) fn create_lookup_texture(
	context: &mut ghi::implementation::Context,
	queue: ghi::QueueHandle,
	builder: ghi::image::Builder,
	pixels: &[u8],
) -> ghi::ImageHandle {
	let extent = builder.get_extent();
	let image_count = extent.depth().max(1) as usize * builder.get_array_layers() as usize;
	let layout = ghi::ImageCopyLayout::new(builder.get_format(), extent, image_count).expect(
		"Lookup texture layout overflowed. The most likely cause is an extent too large for its format. Use a smaller extent.",
	);
	assert_eq!(
		pixels.len(),
		layout.compact_size,
		"Lookup texture '{}' got the wrong number of bytes. The most likely cause is pixels for another extent, format, or layer count. Pass every slice of every layer in compact rows.",
		builder.get_name().unwrap_or("unnamed"),
	);

	let image = context.build_image(builder.additional_uses(ghi::Uses::TransferDestination));
	// The GHI has no buffer destruction yet, so the staging buffer outlives the copy; see `todo.md`.
	let staging = context.build_buffer::<[u8]>(
		ghi::buffer::Builder::new(ghi::Uses::TransferSource)
			.name("Lookup Texture Staging")
			.host_access(ghi::HostAccess::Write)
			.length(layout.padded_size),
	);
	// SAFETY: No GPU work has used the new staging buffer, and the mapping ends before the copy is recorded.
	#[allow(unsafe_code)]
	let staging_bytes = unsafe { context.map_unsynchronized(staging) }.into_bytes();
	layout.write_rows(staging_bytes, pixels, layout.compact_bytes_per_row);
	context.record(queue, Some("Upload Lookup Texture"), |recording| {
		recording.copy_buffer_to_images(&[layout.copy(staging.into(), 0, image.into(), 0)]);
	});
	image
}

/// Uploads [`BLUE_NOISE`] as one RG8 texture whose slices are stacked from the top: slice `s` holds rows `64 s`
/// through `64 s + 63`.
pub(crate) fn create_blue_noise_texture(
	context: &mut ghi::implementation::Context,
	queue: ghi::QueueHandle,
) -> ghi::ImageHandle {
	create_lookup_texture(
		context,
		queue,
		ghi::image::Builder::new(ghi::Formats::RG8UNORM, ghi::Uses::Image)
			.name("Blue Noise")
			.extent(utils::Extent::rectangle(BLUE_NOISE_SIDE, BLUE_NOISE_SIDE * BLUE_NOISE_SLICES)),
		BLUE_NOISE,
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	const SIDE: usize = BLUE_NOISE_SIDE as usize;
	const SLICES: usize = BLUE_NOISE_SLICES as usize;
	const SLICE_TEXELS: usize = SIDE * SIDE;
	const TEXEL_COUNT: usize = SLICE_TEXELS * SLICES;
	/// The standard deviation of both Gaussians that measure how crowded a texel's neighborhood is, in texels across a
	/// slice and in slices through time. Wolfe et al. use 1.9 for every axis.
	const SIGMA: f64 = 1.9;
	/// Kernel weights are rounded to integers at this scale, so energies are exact sums and ties break the same way on
	/// every platform, whatever its `exp` rounds to. Taps that round to 0 are left out.
	const KERNEL_SCALE: f64 = 65536.0;
	const COMMITTED_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/rendering/noise/blue-noise-64x64x32.rg8");

	/// Returns the PCG output permutation of `value` ("Hash Functions for GPU Rendering", Jarzynski and Olano).
	fn hash(value: u32) -> u32 {
		let state = value.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
		let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
		(word >> 22) ^ word
	}

	/// Returns the integer weight of a Gaussian tap `distance_squared` away.
	fn weight(distance_squared: f64) -> i64 {
		((-distance_squared / (2.0 * SIGMA * SIGMA)).exp() * KERNEL_SCALE).round() as i64
	}

	/// Returns the nonzero taps of Wolfe et al.'s spatiotemporal energy as offsets along x, y, and the slices, with
	/// integer weights. Texels of the same slice weigh by their toroidal distance across it, the same texel of other
	/// slices weighs by the toroidal distance through time, and every other pair weighs nothing.
	fn kernel() -> Vec<(isize, isize, isize, i64)> {
		let half = SIDE as isize / 2;
		let mut taps = Vec::new();
		for dy in -half..half {
			for dx in -half..half {
				let weight = weight((dx * dx + dy * dy) as f64);
				if weight > 0 {
					taps.push((dx, dy, 0, weight));
				}
			}
		}
		let half_slices = SLICES as isize / 2;
		for dz in (-half_slices..half_slices).filter(|&dz| dz != 0) {
			let weight = weight((dz * dz) as f64);
			if weight > 0 {
				taps.push((0, 0, dz, weight));
			}
		}
		taps
	}

	/// The `Extremes` struct finds the texel that scores highest among all texels, keeping the answer current as
	/// single scores change, so each void-and-cluster step costs a few hundred tree updates instead of a scan of the
	/// whole volume.
	///
	/// It is a tournament tree: each node holds the better of its two children, and a tie goes to the lower index.
	struct Extremes {
		nodes: Vec<(i64, u32)>,
	}

	impl Extremes {
		fn new() -> Self {
			Self { nodes: (0..2 * TEXEL_COUNT).map(|node| (i64::MIN, (node % TEXEL_COUNT) as u32)).collect() }
		}

		fn better(a: (i64, u32), b: (i64, u32)) -> (i64, u32) {
			if a.0 > b.0 || (a.0 == b.0 && a.1 < b.1) { a } else { b }
		}

		/// Sets `texel`'s score, `i64::MIN` taking it out of the running, and replays its matches up to the root.
		fn set(&mut self, texel: usize, score: i64) {
			let mut node = TEXEL_COUNT + texel;
			self.nodes[node] = (score, texel as u32);
			while node > 1 {
				node /= 2;
				self.nodes[node] = Self::better(self.nodes[2 * node], self.nodes[2 * node + 1]);
			}
		}

		fn best(&self) -> usize {
			self.nodes[1].1 as usize
		}
	}

	/// The `Field` struct is the void-and-cluster state: which texels are points, the energy each texel receives from
	/// the points around it, and the trees that find the tightest cluster and the largest void.
	struct Field {
		taps: Vec<(isize, isize, isize, i64)>,
		pattern: Vec<bool>,
		energy: Vec<i64>,
		/// Scores each point by its energy, so the best is the point with the most crowded neighborhood.
		clusters: Extremes,
		/// Scores each empty texel by its negated energy, so the best is the emptiest neighborhood.
		voids: Extremes,
	}

	impl Field {
		fn new() -> Self {
			let mut voids = Extremes::new();
			for texel in 0..TEXEL_COUNT {
				voids.set(texel, 0);
			}
			Self {
				taps: kernel(),
				pattern: vec![false; TEXEL_COUNT],
				energy: vec![0; TEXEL_COUNT],
				clusters: Extremes::new(),
				voids,
			}
		}

		fn rescore(&mut self, texel: usize) {
			let energy = self.energy[texel];
			let (cluster, void) = if self.pattern[texel] { (energy, i64::MIN) } else { (i64::MIN, -energy) };
			self.clusters.set(texel, cluster);
			self.voids.set(texel, void);
		}

		/// Makes `texel` a point (`on`) or empty, moving the energy it spreads over its neighborhood.
		fn toggle(&mut self, texel: usize, on: bool) {
			self.pattern[texel] = on;
			let sign = if on { 1 } else { -1 };
			let (x, y, z) = ((texel % SIDE) as isize, (texel / SIDE % SIDE) as isize, (texel / SLICE_TEXELS) as isize);
			let (mask, slice_mask) = (SIDE as isize - 1, SLICES as isize - 1);
			for index in 0..self.taps.len() {
				let (dx, dy, dz, weight) = self.taps[index];
				let neighbor = (((z + dz) & slice_mask) * SLICE_TEXELS as isize
					+ ((y + dy) & mask) * SIDE as isize
					+ ((x + dx) & mask)) as usize;
				self.energy[neighbor] += sign * weight;
				self.rescore(neighbor);
			}
			self.rescore(texel);
		}

		fn tightest_cluster(&self) -> usize {
			self.clusters.best()
		}

		fn largest_void(&self) -> usize {
			self.voids.best()
		}
	}

	/// Ranks every texel of the volume by Wolfe et al.'s spatiotemporal void-and-cluster method ("Spatiotemporal Blue
	/// Noise Masks", EGSR 2022), seeded by `seed`. It is Ulichney's method ("The void-and-cluster method for dither array
	/// generation", 1993) under the energy of [`kernel`].
	///
	/// It scatters a tenth of the texels at random, the density Wolfe et al. found best, then moves the point in the
	/// tightest cluster to the largest void until that changes nothing. Points removed from that pattern, tightest
	/// first, take the ranks below its size, and texels added to it, largest void first, the ranks above. Past half the
	/// texels the empty ones are the minority, but the emptiest neighborhood of points is also the most crowded
	/// neighborhood of empty texels, because every texel's two energies sum to the kernel's total, so the same step
	/// finishes the ranking.
	fn void_and_cluster(seed: u32) -> Vec<u32> {
		let mut field = Field::new();
		let initial_count = TEXEL_COUNT / 10;
		let mut placed = 0;
		for counter in 0.. {
			if placed == initial_count {
				break;
			}
			let texel = hash(hash(counter) ^ seed) as usize % TEXEL_COUNT;
			if !field.pattern[texel] {
				field.toggle(texel, true);
				placed += 1;
			}
		}
		loop {
			let cluster = field.tightest_cluster();
			field.toggle(cluster, false);
			let void = field.largest_void();
			field.toggle(void, true);
			if void == cluster {
				break;
			}
		}

		let mut ranks = vec![0u32; TEXEL_COUNT];
		let initial_pattern = field.pattern.clone();
		for rank in (0..initial_count).rev() {
			let cluster = field.tightest_cluster();
			field.toggle(cluster, false);
			ranks[cluster] = rank as u32;
		}
		for texel in (0..TEXEL_COUNT).filter(|&texel| initial_pattern[texel]) {
			field.toggle(texel, true);
		}
		for rank in initial_count..TEXEL_COUNT {
			let void = field.largest_void();
			field.toggle(void, true);
			ranks[void] = rank as u32;
		}
		ranks
	}

	/// Generates [`BLUE_NOISE`]: two spatiotemporal void-and-cluster rankings with different seeds, each rank scaled to
	/// a byte. The two channels generate on their own threads.
	fn generate_blue_noise() -> Vec<u8> {
		let channels = std::thread::scope(|scope| {
			let handles = [0, 1].map(|seed| scope.spawn(move || void_and_cluster(seed)));
			handles.map(|handle| handle.join().unwrap())
		});
		(0..TEXEL_COUNT)
			.flat_map(|texel| {
				channels
					.iter()
					.map(move |ranks| (ranks[texel] as usize * 256 / TEXEL_COUNT) as u8)
			})
			.collect()
	}

	/// Returns channel `channel` of [`BLUE_NOISE`] at texel (`x`, `y`) of slice `slice`, wrapping around every axis.
	fn value(channel: usize, x: usize, y: usize, slice: usize) -> f64 {
		let texel = (slice % SLICES) * SLICE_TEXELS + (y % SIDE) * SIDE + x % SIDE;
		BLUE_NOISE[texel * 2 + channel] as f64
	}

	/// Writes the committed texture. Run it after changing the generator with
	/// `cargo nextest run -p byte-engine --cargo-profile release --run-ignored only write_blue_noise`.
	#[test]
	#[ignore = "writes assets/rendering/noise/blue-noise-64x64x32.rg8"]
	fn write_blue_noise() {
		std::fs::write(COMMITTED_PATH, generate_blue_noise()).unwrap();
	}

	/// Regenerating takes minutes without optimizations, so this runs with the ignored tests.
	#[test]
	#[ignore = "regenerates the blue noise, which takes minutes in a debug build"]
	fn committed_blue_noise_matches_its_generator() {
		assert!(
			generate_blue_noise() == BLUE_NOISE,
			"The committed blue noise differs from what the generator makes. The generator most likely changed without \
			 the file. Run `cargo nextest run -p byte-engine --cargo-profile release --run-ignored only write_blue_noise` \
			 to rewrite it."
		);
	}

	#[test]
	fn blue_noise_uses_every_value_equally_often() {
		for channel in 0..2 {
			let mut counts = [0u32; 256];
			for texel in 0..TEXEL_COUNT {
				counts[BLUE_NOISE[texel * 2 + channel] as usize] += 1;
			}
			assert!(counts.iter().all(|&count| count == 512), "channel {channel}: {counts:?}");
		}
	}

	/// Each slice is blue noise, which has no low frequencies, so every 8x8 window, wrapping around the edges, averages
	/// close to the middle value. White noise strays about 30 from it somewhere in a slice.
	#[test]
	fn every_slice_is_blue_across_the_image() {
		for channel in 0..2 {
			for slice in 0..SLICES {
				for y in 0..SIDE {
					for x in 0..SIDE {
						let sum: f64 = (0..64).map(|tap| value(channel, x + tap % 8, y + tap / 8, slice)).sum();
						let deviation = (sum / 64.0 - 127.5).abs();
						assert!(deviation < 20.0, "channel {channel}, slice {slice}, window at ({x}, {y}): {deviation}");
					}
				}
			}
		}
	}

	/// Each texel's values through the slices are blue noise in time, so every run of 4 consecutive slices, wrapping
	/// around, averages much closer to the middle value than independent values would. Independent values average
	/// with a standard deviation of about 37 over 4 slices.
	#[test]
	fn every_texel_is_blue_through_time() {
		for channel in 0..2 {
			let mut squared_deviation = 0.0;
			let mut runs = 0.0;
			for y in 0..SIDE {
				for x in 0..SIDE {
					for start in 0..SLICES {
						let mean = (0..4).map(|step| value(channel, x, y, start + step)).sum::<f64>() / 4.0;
						squared_deviation += (mean - 127.5).powi(2);
						runs += 1.0;
					}
				}
			}
			let deviation = (squared_deviation / runs).sqrt();
			assert!(deviation < 25.0, "channel {channel}: {deviation}");
		}
	}
}
