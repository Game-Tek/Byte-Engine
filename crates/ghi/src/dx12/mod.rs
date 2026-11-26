pub(crate) mod acceleration_structures;
pub mod command_buffer;
pub mod context;
pub mod device;
pub mod factory;
pub mod frame;
pub mod instance;
mod io;
mod pacing;
pub mod queue;
pub(crate) mod ray_tracing;

pub use self::command_buffer::*;
pub use self::context::*;
pub use self::factory::*;
pub use self::frame::*;
pub use self::instance::*;
pub use self::io::{GDeflateEncoder, ResourceIoQueue, ResourceIoTicket};
pub use self::queue::*;
mod utils;

/// The `Context` type alias exposes the live DX12 device through the cross-backend context name.
pub type Context = self::context::Device;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn dxil_cache_path_changes_with_the_loaded_dxc_commit() {
		let first = Device::hlsl_dxil_cache_path("dxc-1.9-5402-first", "source", "main", "cs_6_9")
			.expect("Expected a DXIL cache path for the first compiler identity");
		let second = Device::hlsl_dxil_cache_path("dxc-1.9-5403-second", "source", "main", "cs_6_9")
			.expect("Expected a DXIL cache path for the second compiler identity");

		assert_ne!(first.file_name(), second.file_name());
	}

	#[test]
	fn agility_sdk_path_uses_the_deployed_default_and_validates_overrides() {
		assert_eq!(Device::agility_sdk_path(None).unwrap().to_bytes(), b".\\D3D12\\");
		assert_eq!(
			Device::agility_sdk_path(Some("C:\\byte-engine\\agility")).unwrap().to_bytes(),
			b"C:\\byte-engine\\agility\\"
		);
		assert_eq!(
			Device::agility_sdk_path(Some("C:\\byte-engine\\agility\\"))
				.unwrap()
				.to_bytes(),
			b"C:\\byte-engine\\agility\\"
		);
		assert!(Device::agility_sdk_path(Some("")).is_err());
		assert!(Device::agility_sdk_path(Some("D3D12\0payload")).is_err());
	}

	#[test]
	fn texture3d_uav_uses_the_selected_mip_depth() {
		let descriptor = Device::descriptor_texture_uav_desc(
			windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM,
			crate::TextureViewTypes::Texture3D,
			::utils::Extent::new(16, 8, 9),
			true,
			1,
			None,
			Some(2),
		);

		assert_eq!(
			descriptor.ViewDimension,
			windows::Win32::Graphics::Direct3D12::D3D12_UAV_DIMENSION_TEXTURE3D
		);
		let texture = unsafe { descriptor.Anonymous.Texture3D };
		assert_eq!(texture.MipSlice, 2);
		assert_eq!(texture.FirstWSlice, 0);
		assert_eq!(texture.WSize, 2);
	}
}
