use std::error::Error;
use std::fmt::{Display, Formatter};

/// Initializes Tracy profiling for applications built on Byte-Engine.
///
/// This should be called by client applications before creating the application, because both
/// `log` and `tracing` allow only one global collector per process.
#[cfg(feature = "tracy")]
pub fn setup_tracy() -> Result<(), TracySetupError> {
	use tracing_subscriber::layer::SubscriberExt;

	// Bridge `log` records into `tracing` events so the Tracy layer receives engine logs too.
	tracing_log::LogTracer::init().map_err(TracySetupError::LogTracer)?;

	let subscriber = tracing_subscriber::registry()
		.with(tracing_subscriber::fmt::layer())
		.with(tracing_tracy::TracyLayer::default());

	tracing::subscriber::set_global_default(subscriber).map_err(TracySetupError::Subscriber)
}

/// Initializes Tracy profiling for applications built on Byte-Engine.
///
/// Enable the `tracy` Cargo feature on `byte-engine` to export spans and logs to Tracy.
#[cfg(not(feature = "tracy"))]
pub fn setup_tracy() -> Result<(), TracySetupError> {
	Err(TracySetupError::Disabled)
}

#[derive(Debug)]
pub enum TracySetupError {
	#[cfg(not(feature = "tracy"))]
	Disabled,
	#[cfg(feature = "tracy")]
	LogTracer(log::SetLoggerError),
	#[cfg(feature = "tracy")]
	Subscriber(tracing::subscriber::SetGlobalDefaultError),
}

impl Display for TracySetupError {
	fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			#[cfg(not(feature = "tracy"))]
			Self::Disabled => formatter.write_str(
				"Failed to set up Tracy profiling. The most likely cause is that byte-engine was built without the `tracy` feature.",
			),
			#[cfg(feature = "tracy")]
			Self::LogTracer(_) => formatter.write_str(
				"Failed to set up Tracy log export. The most likely cause is that another logger was initialized before Tracy setup.",
			),
			#[cfg(feature = "tracy")]
			Self::Subscriber(_) => formatter.write_str(
				"Failed to set up Tracy trace export. The most likely cause is that another tracing subscriber was initialized before Tracy setup.",
			),
		}
	}
}

impl Error for TracySetupError {
	fn source(&self) -> Option<&(dyn Error + 'static)> {
		match self {
			#[cfg(not(feature = "tracy"))]
			Self::Disabled => None,
			#[cfg(feature = "tracy")]
			Self::LogTracer(error) => Some(error),
			#[cfg(feature = "tracy")]
			Self::Subscriber(error) => Some(error),
		}
	}
}
