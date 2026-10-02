//! The authored form of a `.particles` asset.
//!
//! A system is a list of data modules: initialize modules place and launch new particles, update modules change
//! them every frame, and the render section chooses a shape and a color over life. The generator turns the modules
//! into one simulation kernel and one draw pipeline, so a system pays only for the modules it lists.

/// The most particles one system can keep alive.
pub(crate) const MAX_CAPACITY: u32 = 1 << 20;
/// The shortest particle life, in seconds. Shorter lives would age faster than 16 bits of aging rate hold.
const SHORTEST_LIFETIME: f32 = 0.016;
/// The longest particle life, in seconds. Longer lives would round their aging rate to zero.
const LONGEST_LIFETIME: f32 = 1024.0;

/// The `ParticleSystemSource` struct is a `.particles` file as authored.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParticleSystemSource {
	/// Lets editors find the schema; baking ignores it.
	#[serde(rename = "$schema", default)]
	_schema: serde::de::IgnoredAny,
	pub(crate) capacity: u32,
	/// The shortest and longest life of a particle, in seconds.
	pub(crate) lifetime: [f32; 2],
	#[serde(default)]
	pub(crate) spawn: SpawnSource,
	#[serde(default)]
	pub(crate) initialize: Vec<InitializeModule>,
	#[serde(default)]
	pub(crate) update: Vec<UpdateModule>,
	pub(crate) render: RenderSource,
}

/// The `SpawnSource` struct sets how much an emitter spawns when it does not choose for itself.
#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpawnSource {
	/// Particles per second.
	#[serde(default)]
	pub(crate) rate: f32,
	/// Particles spawned once, when an emitter is published.
	#[serde(default)]
	pub(crate) burst: u32,
}

/// The `InitializeModule` enum is one step that places or launches a new particle in its emitter's space.
///
/// Modules run in the listed order and add up: two velocity modules add their velocities.
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum InitializeModule {
	/// Starts particles at a uniformly random point inside a sphere around the emitter.
	Sphere { radius: f32 },
	/// Starts particles at a uniformly random point inside a box centered on the emitter, such as a rain cloud.
	Box {
		/// The box's width, height, and depth along the emitter's local axes, in meters.
		size: [f32; 3],
	},
	/// Launches particles in a uniformly random direction inside a cone around the emitter's local `+Z` axis.
	Cone {
		/// The cone's half angle, in radians.
		angle: f32,
		/// The slowest and fastest launch speed, in meters per second.
		speed: [f32; 2],
	},
}

/// The `UpdateModule` enum is one step that changes every live particle each frame.
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum UpdateModule {
	/// Accelerates particles by a world-space vector in meters per second squared, such as gravity or wind.
	Acceleration { value: [f32; 3] },
	/// Slows particles by `coefficient`, the fraction of speed they lose per second at low speed.
	Drag { coefficient: f32 },
}

/// The `RenderSource` struct sets how a particle looks.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RenderSource {
	pub(crate) shape: Shape,
	/// The light a particle emits over its life, as stops interpolated linearly by age.
	pub(crate) color: Vec<ColorStop>,
}

/// The `Shape` enum chooses the camera-facing quad each particle draws as.
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Shape {
	/// A streak along the particle's velocity, for fast particles such as sparks.
	Streak {
		/// The streak's width, in meters.
		width: f32,
		/// How many seconds of travel the streak shows behind the particle.
		stretch: f32,
	},
	/// A round sprite of `size` meters across.
	Billboard { size: f32 },
}

/// The `ColorStop` struct is the emitted light at one point of a particle's life.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ColorStop {
	/// The fraction of its life the particle has lived, from `0` at birth to `1` at death.
	pub(crate) age: f32,
	/// Linear sRGB luminance in nits. Particles add this light to the scene.
	pub(crate) radiance: [f32; 3],
}

impl ParticleSystemSource {
	/// Rejects values the kernels cannot run, returning a message that names the field.
	pub(crate) fn validate(&self) -> Result<(), String> {
		if self.capacity == 0 || self.capacity > MAX_CAPACITY {
			return Err(format!(
				"`capacity` must be between 1 and {MAX_CAPACITY}, but it is {}.",
				self.capacity
			));
		}
		let [shortest, longest] = self.lifetime;
		if !(SHORTEST_LIFETIME..=LONGEST_LIFETIME).contains(&shortest) || !(shortest..=LONGEST_LIFETIME).contains(&longest) {
			return Err(format!(
				"`lifetime` must be two increasing values between {SHORTEST_LIFETIME} and {LONGEST_LIFETIME} seconds."
			));
		}
		if !(self.spawn.rate.is_finite() && self.spawn.rate >= 0.0) {
			return Err("`spawn.rate` must be a finite value of at least zero.".to_string());
		}
		for module in &self.initialize {
			match module {
				InitializeModule::Sphere { radius } => non_negative("sphere `radius`", *radius)?,
				InitializeModule::Box { size } => {
					for side in size {
						non_negative("box `size`", *side)?;
					}
				}
				InitializeModule::Cone { angle, speed } => {
					if !(0.0..=std::f32::consts::PI).contains(angle) {
						return Err("cone `angle` must be between 0 and π radians.".to_string());
					}
					increasing("cone `speed`", *speed)?;
				}
			}
		}
		for module in &self.update {
			match module {
				UpdateModule::Acceleration { value } => {
					if !value.iter().all(|component| component.is_finite()) {
						return Err("acceleration `value` must be finite.".to_string());
					}
				}
				UpdateModule::Drag { coefficient } => non_negative("drag `coefficient`", *coefficient)?,
			}
		}
		match self.render.shape {
			Shape::Streak { width, stretch } => {
				positive("streak `width`", width)?;
				non_negative("streak `stretch`", stretch)?;
			}
			Shape::Billboard { size } => positive("billboard `size`", size)?,
		}
		if self.render.color.is_empty() {
			return Err("`render.color` needs at least one stop.".to_string());
		}
		let mut previous_age = f32::NEG_INFINITY;
		for stop in &self.render.color {
			if !(0.0..=1.0).contains(&stop.age) || stop.age <= previous_age {
				return Err("`render.color` stop ages must increase strictly from 0 to 1.".to_string());
			}
			previous_age = stop.age;
			for component in stop.radiance {
				non_negative("color stop `radiance`", component)?;
			}
		}
		Ok(())
	}
}

fn non_negative(name: &str, value: f32) -> Result<(), String> {
	if value.is_finite() && value >= 0.0 {
		Ok(())
	} else {
		Err(format!("{name} must be a finite value of at least zero."))
	}
}

fn positive(name: &str, value: f32) -> Result<(), String> {
	if value.is_finite() && value > 0.0 {
		Ok(())
	} else {
		Err(format!("{name} must be a finite value above zero."))
	}
}

fn increasing(name: &str, [low, high]: [f32; 2]) -> Result<(), String> {
	non_negative(name, low)?;
	non_negative(name, high)?;
	if low <= high {
		Ok(())
	} else {
		Err(format!("{name} must list its smaller value first."))
	}
}
