use crate::raster_pipeline;

pub trait Factory {
	/// Creates a graphics/rasterization pipeline from a builder.
	fn create_raster_pipeline(&mut self, builder: raster_pipeline::Builder);
}
