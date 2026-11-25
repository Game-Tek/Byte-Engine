use utils::Extent;

use crate::{Formats, Size as _};

pub(crate) fn texture_copy_layout(format: Formats, extent: Extent) -> Option<(usize, usize, usize)> {
	Some(format.compact_copy_layout(extent.width(), extent.height()))
}

pub(crate) fn bytes_per_pixel(format: Formats) -> Option<usize> {
	format.bc_bytes_per_block().is_none().then(|| format.size())
}
