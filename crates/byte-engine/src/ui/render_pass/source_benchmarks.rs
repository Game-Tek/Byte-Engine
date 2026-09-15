//! Exercise production source resolution; the original scan benchmark stays unchanged.

use divan::{Bencher, black_box};

use super::*;

/// Resolves generated batches for distinct images, without texture creation or upload.
#[divan::bench(args = [100, 1000])]
fn image_sources(bencher: Bencher, count: usize) {
	let data = UiDrawList {
		layout_size: [1920.0, 1080.0],
		images: (0..count)
			.map(|index| UiImageDrawElement {
				depth: (index % 4) as u32,
				order: index as u32,
				image_id: index as u64,
				version: 0,
				source_width: 4,
				source_height: 4,
				pixels: vec![255; 64].into(),
				position: [(index % 40 * 48) as f32, (index / 40 * 40) as f32],
				size: [24.0, 24.0],
				clip: None,
				feather_mask: None,
				opacity: 1.0,
			})
			.collect(),
		..UiDrawList::default()
	};
	let arena = bumpalo::Bump::new();
	let geometry = build_ui_image_geometry(&data, Extent::rectangle(1920, 1080), &arena);
	assert_eq!(geometry.batches.len(), count);
	bencher.bench_local(|| {
		for batch in black_box(&geometry.batches) {
			black_box(batch.source(black_box(&data.images)).unwrap());
		}
	});
}
