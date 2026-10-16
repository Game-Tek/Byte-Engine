//! The physics system an application steps on its world scope.

use std::ops::{Deref, DerefMut};

use crate::{
	application::{Time, parameters::Parameters},
	core::{
		alley::Alley,
		channel::DefaultChannel,
		listener::{DefaultListener, Listener},
		message_bus::MessageScope,
	},
	gameplay::{World as _, transform::TransformationUpdate},
	physics::dynabit,
};

/// The `Physics` struct simulates the bodies of a world scope and publishes where they move on its transform route.
///
/// Create it with [`Self::new`] before the bodies it must simulate, then call [`Self::step`] once per
/// simulation step, after everything that posts transforms for that step, and [`Self::flush_deletions`] where
/// deleted bodies should leave the simulation, such as the start of the next step. It dereferences to the
/// [`dynabit::World`] it steps for settings such as [`dynabit::World::set_sleeping`].
///
/// # Configuration
/// Read from the parameters the caller routes to it, usually `app.parameters().scope("physics")`:
/// - `step-lanes`: Replaces how many lanes a step runs on, as `bodies:lanes` pairs such as `512:2,4000:4`: steps
///   of at least 512 bodies run on two lanes and steps of at least 4,000 on four. Smaller steps, and steps given
///   one lane, run on the calling thread, so `0:1` keeps every step there. The default suits the machine: it never
///   uses more lanes than the processor's fastest cores have hardware threads; see
///   [`StepLanes::for_machine`](crate::physics::StepLanes::for_machine). Set it per machine, such as with the
///   `BE_PHYSICS__STEP_LANES` environment variable, when measurements there show better counts.
pub struct Physics {
	world: dynabit::World,
	transforms: DefaultChannel<TransformationUpdate>,
	/// Reads the transforms authored for a step, and afterwards the ones the step published, so the step's
	/// own results never come back as authored transforms.
	authored: DefaultListener<TransformationUpdate>,
}

impl Physics {
	/// Creates a simulation of the bodies, constraints, and ray queries published in `scope` from now on, set up
	/// from `parameters`, the physics subtree of the application's.
	pub fn new(scope: &MessageScope, parameters: impl Parameters) -> Self {
		let transforms = scope.transforms_channel();
		let authored = transforms.listener();
		let mut world = dynabit::World::new(
			scope.factory().listener(),
			scope.factory().listener(),
			scope.deletions_listener(),
			scope.factory().listener(),
			scope.factory(),
		);
		// A typo in a setting meant for this machine stops startup instead of silently leaving the defaults in place.
		if let Some(step_lanes) = parameters.get_parameter("step-lanes") {
			world.set_step_lanes(step_lanes.parse().unwrap_or_else(|error| panic!("{error}")));
		}
		Self {
			world,
			transforms,
			authored,
		}
	}

	/// Advances the simulation by `time`, running the step as one execution of `alley`.
	///
	/// The step runs on as many of `alley`'s lanes as its size pays for; see [`dynabit::World::update`]. No other
	/// dispatch of `alley` may be running, so call it outside lane executions such as
	/// [`GraphicsApplication::run_lanes`](crate::application::graphics::GraphicsApplication::run_lanes).
	pub fn step(&mut self, time: Time, alley: &mut Alley) {
		self.world.update(time, &mut self.authored, &self.transforms, alley);
		// Physics results return on the shared transform route; drop them so the next step collects only
		// authored transforms.
		while self.authored.read().is_some() {}
	}

	/// Removes the bodies and constraints of every deletion published since the last call.
	///
	/// Call it where other systems have finished reading the deleted bodies, such as the start of a step.
	pub fn flush_deletions(&mut self) {
		self.world.process_pending_deletions();
	}
}

impl Deref for Physics {
	type Target = dynabit::World;

	fn deref(&self) -> &Self::Target {
		&self.world
	}
}

impl DerefMut for Physics {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.world
	}
}
