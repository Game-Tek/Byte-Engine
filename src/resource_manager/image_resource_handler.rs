use polodb_core::bson::Document;
use serde::{Serialize, Deserialize};

use super::ResourceHandler;

pub(crate) struct ImageResourceHandler {

}

impl ImageResourceHandler {
	pub fn new() -> Self {
		Self {}
	}
}

impl ResourceHandler for ImageResourceHandler {
	fn can_handle_type(&self, resource_type: &str) -> bool {
		match resource_type {
			"image/png" => true,
			"png" => true,
			_ => false
		}
	}

	fn process(&self, bytes: &[u8]) -> Result<Vec<(Document, Vec<u8>)>, String> {
		let mut decoder = png::Decoder::new(bytes);
		decoder.set_transformations(png::Transformations::EXPAND);
		let mut reader = decoder.read_info().unwrap();
		let mut buffer = vec![0; reader.output_buffer_size()];
		let info = reader.next_frame(&mut buffer).unwrap();

		let extent = crate::Extent { width: info.width, height: info.height, depth: 1, };

		let mut buf: Vec<u8> = Vec::with_capacity(extent.width as usize * extent.height as usize * 4);

		// convert rgb to rgba
		for x in 0..extent.width {
			for y in 0..extent.height {
				let index = ((x + y * extent.width) * 3) as usize;
				buf.push(buffer[index]);
				buf.push(buffer[index + 1]);
				buf.push(buffer[index + 2]);
				buf.push(255);
			}
		}

		assert_eq!(extent.depth, 1); // TODO: support 3D textures

		let rgba_surface = intel_tex_2::RgbaSurface {
			data: &buf,
			width: extent.width,
			height: extent.height,
			stride: extent.width * 4,
		};

		let settings = intel_tex_2::bc7::opaque_ultra_fast_settings();

		let resource_document = polodb_core::bson::doc!{
			"class": "Texture",
			"resource": {
				"extent": [extent.width, extent.height, extent.depth],
				"compression": "BC7",
			}
		};

		Ok(vec![(resource_document, intel_tex_2::bc7::compress_blocks(&settings, &rgba_surface))])
	}

	fn get_deserializer(&self) -> Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn std::any::Any> + Send> {
		Box::new(|document| {
			let texture = Texture::deserialize(polodb_core::bson::Deserializer::new(document.into())).unwrap();
			Box::new(texture)
		})
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Texture {
	pub compression: String,
	pub extent: crate::Extent,
}