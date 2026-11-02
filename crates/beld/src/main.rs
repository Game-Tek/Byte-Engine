//! BELD with the engine's default asset handlers, for applications that add no parameters or formats of their own.

fn main() -> std::process::ExitCode {
	beld::run(&[], byte_engine::application::graphics::configure_default_baking)
}
