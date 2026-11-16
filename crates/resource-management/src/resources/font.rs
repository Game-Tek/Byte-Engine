/// The `Font` struct lets an application draw text with a font it ships as a resource instead of a system font.
///
/// The model is empty: the payload holds the TrueType or OpenType file unchanged. Request one through
/// [`crate::ResourceManager::request`] and read its payload with [`crate::Reference::load`], then give the bytes to the
/// UI.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Font {}

super::impl_direct_resource!(Font, "Font");
