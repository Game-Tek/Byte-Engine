use crate::pipelines;

pub trait Factory {
	/// Creates a graphics/rasterization pipeline from a builder.
	fn create_raster_pipeline(&mut self, builder: pipelines::raster::Builder);
}
