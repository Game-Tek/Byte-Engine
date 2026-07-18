pub(crate) mod artifact;
pub mod besl;
pub mod generator;
#[cfg(target_os = "linux")]
pub mod glsl_compile;
pub(crate) mod hlsl_shader_compiler;
#[cfg(target_os = "macos")]
pub mod msl_shader_compiler;

pub use generator::Generator as ShaderGenerator;
pub use generator::Settings as ShaderGenerationSettings;
