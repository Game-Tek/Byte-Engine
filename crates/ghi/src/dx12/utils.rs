use utils::Extent;

use crate::{Formats, Size as _};

pub(crate) fn texture_copy_layout(format: Formats, extent: Extent) -> Option<(usize, usize, usize)> {
	Some(format.compact_copy_layout(extent.width(), extent.height()))
}

pub(crate) fn texture_copy_size(format: Formats, extent: Extent) -> Option<usize> {
	let (_, _, bytes_per_image) = texture_copy_layout(format, extent)?;
	Some(bytes_per_image * extent.depth().max(1) as usize)
}

pub(crate) fn bytes_per_pixel(format: Formats) -> Option<usize> {
	format.bc_bytes_per_block().is_none().then(|| format.size())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn texture_copy_layout_preserves_raw_zero_extent() {
		assert_eq!(
			texture_copy_layout(Formats::RGBA8UNORM, Extent::rectangle(0, 0)),
			Some((0, 0, 0))
		);
	}
}
