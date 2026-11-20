//! Exercises temporal upscalers: a still, evenly colored frame upscales to the same color at the output extent.

use super::*;

const INPUT: Extent = Extent::rectangle(64, 64);
const OUTPUT: Extent = Extent::rectangle(128, 128);
/// A color every format here holds exactly.
const GRAY: f32 = 0.5;

/// Returns the unsigned float that `bits` holds with `mantissa_bits` of mantissa and a 5-bit exponent, as the 11- and
/// 10-bit channels of `RGBu11u11u10` store it.
fn small_float(bits: u32, mantissa_bits: u32) -> f32 {
	let exponent = (bits >> mantissa_bits) as i32;
	let mantissa = (bits & ((1 << mantissa_bits) - 1)) as f32 / (1 << mantissa_bits) as f32;
	match exponent {
		0 => mantissa * 2f32.powi(-14),
		_ => (1.0 + mantissa) * 2f32.powi(exponent - 15),
	}
}

/// Decodes an `RGBu11u11u10` readback into its red, green, and blue channels.
fn rgb_pixels(readback: ghi::TextureReadback) -> Vec<[f32; 3]> {
	readback
		.bytes
		.as_chunks::<4>()
		.0
		.iter()
		.map(|texel| {
			let packed = u32::from_le_bytes(*texel);
			[
				small_float(packed & 0x7ff, 6),
				small_float((packed >> 11) & 0x7ff, 6),
				small_float(packed >> 22, 5),
			]
		})
		.collect()
}

/// Upscales a gray, still frame several frames in a row, writing every input in the same recording first, so the
/// upscaler must wait for those writes. The output is that gray everywhere.
///
/// Uses the formats the engine's temporal antialiasing pass hands it: `RGBu11u11u10` color and output, reversed
/// `Depth32` depth, `RGBA16F` motion, and an `R8UNORM` reactive mask. A device without a temporal upscaler skips it.
pub(super) fn a_still_frame_upscales_to_its_own_color(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	let Some(scales) = device.temporal_upscaler_scale_range() else {
		eprintln!("Skipping: the device has no temporal upscaler.");
		return;
	};
	assert!(
		scales.contains(&0.5),
		"The device's temporal upscaler doesn't take a 2x upscale ({scales:?}). The most likely cause is a GPU that only upscales by less. Run this test on an Apple silicon Mac."
	);
	let mut input = |format: Formats, name: &str| {
		device.build_image(
			ghi::image::Builder::new(format, Uses::Image | Uses::Clear)
				.name(name)
				.extent(INPUT),
		)
	};
	let color = input(Formats::RGBu11u11u10, "Upscaler Color");
	let depth = input(Formats::Depth32, "Upscaler Depth");
	let motion = input(Formats::RGBA16F, "Upscaler Motion");
	let reactive = input(Formats::R8UNORM, "Upscaler Reactive");
	let output = device.build_image(
		ghi::image::Builder::new(
			Formats::RGBu11u11u10,
			Uses::Image | Uses::Storage | Uses::RenderTarget | Uses::Clear | Uses::TransferSource,
		)
		.name("Upscaler Output")
		.extent(OUTPUT),
	);
	let upscaler = device
		.build_temporal_upscaler(
			ghi::upscaler::Builder::new(
				INPUT,
				OUTPUT,
				[Formats::RGBu11u11u10, Formats::Depth32, Formats::RGBA16F],
				Formats::RGBu11u11u10,
			)
			.name("Test Upscaler")
			.reactive_format(Formats::R8UNORM)
			.depth_reversed(true),
		)
		.unwrap_or_else(|error| panic!("{error}"));

	let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);
	let synchronizer = device.create_synchronizer(None, true);
	let mut readback = None;
	let frames = 4u64;
	for frame_index in 0..frames {
		device.queue(queue_handle).execute(
			Some(FrameRequest::new(frame_index, synchronizer)),
			&[],
			synchronizer,
			|execution| {
				execution.record(command_buffer_handle, |recording| {
					let gray = RGBA {
						r: GRAY,
						g: GRAY,
						b: GRAY,
						a: 1.0,
					};
					recording.clear_images(&[
						(color.into(), ClearValue::Color(gray)),
						(depth.into(), ClearValue::Depth(0.0)),
						(motion.into(), ClearValue::Color(RGBA::black())),
						(reactive.into(), ClearValue::Color(RGBA::black())),
						(output.into(), ClearValue::Color(RGBA::black())),
					]);
					recording.upscale(
						upscaler,
						&ghi::upscaler::Inputs {
							color: color.into(),
							depth: depth.into(),
							motion: motion.into(),
							reactive: Some(reactive.into()),
							output: output.into(),
							content_extent: INPUT,
							jitter: [0.0, 0.0],
							motion_scale: [-1.0, -1.0],
							pre_exposure: 1.0,
							reset: frame_index == 0,
						},
					);
					if frame_index == frames - 1 {
						readback = Some(recording.transfer_texture(output.into()).expect(
							"Texture transfer failed. The most likely cause is that the output isn't a transfer source.",
						));
					}
				});
				[]
			},
		);
	}
	device.wait();
	assert!(!device.has_errors());

	let pixels = rgb_pixels(
		device
			.get_image_data(readback.expect("The last frame records the readback."))
			.expect(
				"Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.",
			),
	);
	assert_eq!(pixels.len(), (OUTPUT.width() * OUTPUT.height()) as usize);
	let worst = pixels
		.iter()
		.flatten()
		.map(|channel| (channel - GRAY).abs())
		.fold(0.0f32, f32::max);
	assert!(
		worst < 0.02,
		"The upscaled gray strays by {worst} from {GRAY}. The most likely cause is the upscaler reading its inputs before the clears finished, or writing nothing."
	);
}
