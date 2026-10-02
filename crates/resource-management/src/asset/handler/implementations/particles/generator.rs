//! Turns a particle system's modules into BESL.
//!
//! Every system shares one kernel shape: the renderer's prepare pass sizes the simulation, the simulation spawns,
//! moves, and packs the survivors into the other half of a ping-pong buffer, and an indirect draw expands each
//! particle into a quad. Modules only fill in the spawn, update, shape, and color steps, with their values baked in
//! as constants, so a system runs no code for modules it does not list.
//!
//! The data the renderer writes is a contract: `ParticleFrame`, the bindings, and the push constant here must match
//! `rendering/particles/shader_data.rs` in the engine and its `prepare.besl`.

use std::fmt::Write as _;

use super::schema::{ColorStop, InitializeModule, ParticleSystemSource, Shape, UpdateModule};

/// Threads per simulation workgroup.
pub(crate) const SIMULATION_WORKGROUP_SIZE: u32 = 64;
/// The bytes of the draw pipeline's push constant: a view-projection matrix and the camera position and exposure.
pub(crate) const DRAW_PUSH_CONSTANT_SIZE: u32 = 80;
/// The steps per second a particle's aging rate is stored in, 16 bits of `packed`.
const DECAY_STEPS: f32 = 1024.0;

/// The `ParticlePrograms` struct holds the generated BESL source of one particle system.
pub(crate) struct ParticlePrograms {
	pub(crate) simulate: String,
	pub(crate) vertex: String,
	pub(crate) fragment: String,
}

/// Declarations the simulation and vertex stages share with the renderer.
///
/// Particles store only scalars, because backends pad a `vec3f` struct member to 16 bytes and would double the
/// stride. `packed` holds the emitter slot in its low 16 bits and the life lost per second, in [`DECAY_STEPS`]
/// steps, in its high 16 bits.
const SHARED_DECLARATIONS: &str = "Spawn: struct {
	first: u32,
	emitter: u32,
}

Emitter: struct {
	model: mat4x3f,
}

ParticleFrame: struct {
	delta_time: f32,
	seed: u32,
	spawn_total: u32,
	spawn_count: u32,
	side: u32,
	reset: u32,
	capacity: u32,
	padding: u32,
	spawns: Spawn[256],
	emitters: Emitter[256],
}

Particle: struct {
	position_x: f32,
	position_y: f32,
	position_z: f32,
	life: f32,
	velocity_x: f32,
	velocity_y: f32,
	velocity_z: f32,
	packed: u32,
}
";

/// Returns the longest a particle of a validated system can live, after the kernel rounds its aging rate down.
pub(crate) fn longest_life(system: &ParticleSystemSource) -> f32 {
	DECAY_STEPS / (DECAY_STEPS / system.lifetime[1]).floor()
}

/// Generates the simulation, vertex, and fragment programs of a validated system.
pub(crate) fn generate(system: &ParticleSystemSource) -> ParticlePrograms {
	ParticlePrograms {
		simulate: simulate_program(system),
		vertex: vertex_program(system),
		fragment: fragment_program(&system.render.shape),
	}
}

fn simulate_program(system: &ParticleSystemSource) -> String {
	// Random draws 0 and 1 belong to the lifetime and the birth moment; modules take theirs after them.
	let mut next_random = 2u32;
	let mut initialize = String::new();
	for (index, module) in system.initialize.iter().enumerate() {
		let mut random = || {
			let key = next_random;
			next_random += 1;
			format!("unit(hash(seed + {key}))")
		};
		let _ = match module {
			InitializeModule::Sphere { radius } => write!(
				initialize,
				"		let m{index}_z: f32 = 1.0 - 2.0 * {};
		let m{index}_around: vec2f = sincos(6.2831853 * {});
		let m{index}_ring: f32 = sqrt(max(1.0 - m{index}_z * m{index}_z, 0.0));
		let m{index}_distance: f32 = {} * pow({}, 0.3333333);
		offset = offset + vec3f(m{index}_ring * m{index}_around.y, m{index}_ring * m{index}_around.x, m{index}_z) * m{index}_distance;
",
				random(),
				random(),
				literal(*radius),
				random(),
			),
			InitializeModule::Box { size: [width, height, depth] } => writeln!(
				initialize,
				"		offset = offset + vec3f(({} - 0.5) * {}, ({} - 0.5) * {}, ({} - 0.5) * {});",
				random(),
				literal(*width),
				random(),
				literal(*height),
				random(),
				literal(*depth),
			),
			InitializeModule::Cone { angle, speed } => write!(
				initialize,
				"		let m{index}_cos: f32 = mix(1.0, {}, {});
		let m{index}_sin: f32 = sqrt(max(1.0 - m{index}_cos * m{index}_cos, 0.0));
		let m{index}_around: vec2f = sincos(6.2831853 * {});
		let m{index}_speed: f32 = mix({}, {}, {});
		launch = launch + vec3f(m{index}_sin * m{index}_around.y, m{index}_sin * m{index}_around.x, m{index}_cos) * m{index}_speed;
",
				literal(angle.cos()),
				random(),
				random(),
				literal(speed[0]),
				literal(speed[1]),
				random(),
			),
		};
	}

	let mut update = String::new();
	for module in &system.update {
		let _ = match module {
			UpdateModule::Acceleration { value } => writeln!(update, "		velocity = velocity + {} * step;", vector(*value)),
			UpdateModule::Drag { coefficient } => {
				writeln!(update, "		velocity = velocity / (1.0 + {} * step);", literal(*coefficient))
			}
		};
	}

	format!(
		"// Generated from a `.particles` asset. Spawns this frame's new particles, advances every live particle, and
// packs the survivors into the other half of the particle buffer. Each workgroup reserves its survivors' slots with
// one atomic on the draw record, so the draw's vertex count is also the next frame's live count.

{SHARED_DECLARATIONS}
ParticleDraws: struct {{
	values: atomicu32[8],
}}

frame: descriptor<{{ type: ParticleFrame, binding: 0, access: read }}>;
particles: descriptor<{{ type: Particle[], binding: 1, access: read_write }}>;
draws: descriptor<{{ type: ParticleDraws, binding: 2, access: read_write }}>;

group_count: workgroup<atomicu32>;
group_base: workgroup<atomicu32>;

DECAY_STEPS: const f32 = {decay_steps};

// PCG hash: a cheap, well-mixed 32-bit permutation.
hash: fn (value: u32) -> u32 {{
	let state: u32 = value * 747796405 + 2891336453;
	let word: u32 = ((state >> ((state >> 28) + 4)) ^ state) * 277803737;
	return (word >> 22) ^ word;
}}

// Maps a hash to [0, 1) using its 24 high bits, which a float holds exactly.
unit: fn (value: u32) -> f32 {{
	return f32(value >> 8) / 16777216.0;
}}

main: fn (input: StageInput) -> void {{
	let index: u32 = input.thread_id.x;
	let previous: u32 = (frame.side + 1) % 2;
	let alive: u32 = 0;
	if (frame.reset == 0) {{
		alive = atomic_load(draws.values[previous * 4]) / 6;
	}}
	// The same clamp as the prepare pass, so both agree on how many particles this dispatch covers.
	let emitted: u32 = min(frame.spawn_total, frame.capacity - alive);

	if (input.thread_idx == 0) {{
		atomic_store(group_count, 0);
	}}
	workgroup_barrier();

	let position: vec3f = vec3f(0.0, 0.0, 0.0);
	let velocity: vec3f = vec3f(0.0, 0.0, 0.0);
	let life: f32 = 0.0;
	let packed: u32 = 0;
	let step: f32 = frame.delta_time;

	if (index < alive) {{
		let particle: Particle = particles[previous * frame.capacity + index];
		position = vec3f(particle.position_x, particle.position_y, particle.position_z);
		velocity = vec3f(particle.velocity_x, particle.velocity_y, particle.velocity_z);
		life = particle.life;
		packed = particle.packed;
	}} else if (index < alive + emitted) {{
		let spawn_index: u32 = index - alive;

		// The last spawn run that starts at or before this particle owns it. 256 runs need at most 8 halvings.
		let low: u32 = 0;
		let high: u32 = frame.spawn_count;
		for (let halving: u32 = 0; halving < 8; halving = halving + 1) {{
			if (high - low > 1) {{
				let middle: u32 = (low + high) / 2;
				if (frame.spawns[middle].first <= spawn_index) {{
					low = middle;
				}} else {{
					high = middle;
				}}
			}}
		}}
		let slot: u32 = frame.spawns[low].emitter;
		let seed: u32 = hash(spawn_index ^ hash(frame.seed));

		// Modules work in the emitter's space; its transform moves, turns, and scales the result.
		let offset: vec3f = vec3f(0.0, 0.0, 0.0);
		let launch: vec3f = vec3f(0.0, 0.0, 0.0);
{initialize}
		let lifetime: f32 = mix({lifetime_min}, {lifetime_max}, unit(hash(seed)));
		// Validated lifetimes keep this between 1 and what 16 bits hold, so no particle lives forever.
		let decay_steps: u32 = u32(DECAY_STEPS / lifetime);

		position = frame.emitters[slot].model * vec4f(offset.x, offset.y, offset.z, 1.0);
		velocity = frame.emitters[slot].model * vec4f(launch.x, launch.y, launch.z, 0.0);
		life = 1.0;
		packed = slot | (decay_steps << 16);
		// Each particle was born at a random moment of the frame, so a steady stream does not leave in clumps.
		step = frame.delta_time * unit(hash(seed + 1));
	}}

	let keep: bool = false;
	if (life > 0.0) {{
{update}		position = position + velocity * step;
		life = life - f32(packed >> 16) / DECAY_STEPS * step;
		keep = life > 0.0;
	}}

	let local_index: u32 = 0;
	if (keep) {{
		local_index = atomic_add(group_count, 1);
	}}
	workgroup_barrier();

	// One global atomic per workgroup reserves the group's survivors and grows the draw at the same time.
	if (input.thread_idx == 0) {{
		let count: u32 = atomic_load(group_count);
		if (count > 0) {{
			atomic_store(group_base, atomic_add(draws.values[frame.side * 4], count * 6) / 6);
		}}
	}}
	workgroup_barrier();

	if (keep) {{
		let destination: u32 = frame.side * frame.capacity + atomic_load(group_base) + local_index;
		particles[destination] = Particle(position.x, position.y, position.z, life, velocity.x, velocity.y, velocity.z, packed);
	}}
}}
",
		lifetime_min = literal(system.lifetime[0]),
		lifetime_max = literal(system.lifetime[1]),
		decay_steps = literal(DECAY_STEPS),
	)
}

fn vertex_program(system: &ParticleSystemSource) -> String {
	let shape = match system.render.shape {
		Shape::Streak { width, stretch } => format!(
			"	// The streak covers the distance the particle travels in its stretch time, so faster particles draw longer.
	let trail: vec3f = velocity * {stretch};
	let trail_length: f32 = length(trail);
	let axis: vec3f = vec3f(0.0, 1.0, 0.0);
	if (trail_length > 0.00001) {{
		axis = trail / trail_length;
	}}
	// A particle moving straight at the camera has no screen-space direction, so any perpendicular will do.
	let across: vec3f = cross(axis, to_camera);
	if (dot(across, across) < 0.00000001) {{
		across = cross(axis, vec3f(1.0, 0.0, 0.0));
	}}
	across = normalize(across);
	// Half a width of cap on each end keeps slow particles round instead of collapsing to a line.
	let head: vec3f = center + axis * {half_width};
	let tail: vec3f = center - trail - axis * {half_width};
	let world_position: vec3f = tail + (head - tail) * corner.x + across * ({width} * (corner.y - 0.5));
",
			stretch = literal(stretch),
			half_width = literal(width * 0.5),
			width = literal(width),
		),
		Shape::Billboard { size } => format!(
			"	// Any up direction works for a round sprite; fall back when looking straight along the first choice.
	let right: vec3f = cross(to_camera, vec3f(0.0, 1.0, 0.0));
	if (dot(right, right) < 0.00000001) {{
		right = cross(to_camera, vec3f(1.0, 0.0, 0.0));
	}}
	right = normalize(right);
	let up: vec3f = normalize(cross(right, to_camera));
	let world_position: vec3f = center + right * ({size} * (corner.x - 0.5)) + up * ({size} * (corner.y - 0.5));
",
			size = literal(size),
		),
	};

	format!(
		"// Generated from a `.particles` asset. Draws each live particle as a camera-facing quad. No vertex buffer is
// bound: six vertices pull one particle from the half of the particle buffer the simulation just filled.

{SHARED_DECLARATIONS}
frame: descriptor<{{ type: ParticleFrame, binding: 0, access: read }}>;
particles: descriptor<{{ type: Particle[], binding: 1, access: read }}>;

push_constant: push_constant {{
	view_projection: mat4f,
	// The camera position in `xyz` and the camera exposure in `w`.
	camera: vec4f,
}}

main: fn (input: StageInput) -> interface {{ position: vec4f, radiance: vec3f, uv: vec2f }} {{
	let particle: Particle = particles[frame.side * frame.capacity + input.vertex_index / 6];
	// Two triangles: x runs from the tail to the head and y across the quad.
	let corner_index: u32 = input.vertex_index % 6;
	let corner: vec2f = vec2f(f32((14 >> corner_index) & 1), f32((28 >> corner_index) & 1));

	let center: vec3f = vec3f(particle.position_x, particle.position_y, particle.position_z);
	let velocity: vec3f = vec3f(particle.velocity_x, particle.velocity_y, particle.velocity_z);
	let to_camera: vec3f = vec3f(push_constant.camera.x, push_constant.camera.y, push_constant.camera.z) - center;
{shape}	let position: vec4f = push_constant.view_projection * vec4f(world_position.x, world_position.y, world_position.z, 1.0);

	let age: f32 = 1.0 - clamp(particle.life, 0.0, 1.0);
{color}
	return {{ position, radiance: color * push_constant.camera.w, uv: vec2f(corner.x * 2.0 - 1.0, corner.y * 2.0 - 1.0) }};
}}
",
		color = color_gradient(&system.render.color),
	)
}

/// Builds the color stops into a chain of linear segments. Each later segment overrides the earlier ones once the
/// age passes its start, so the last stop's color holds after it and the first stop's before it.
fn color_gradient(stops: &[ColorStop]) -> String {
	let mut code = format!("	let color: vec3f = {};\n", vector(stops[0].radiance));
	for pair in stops.windows(2) {
		let (from, to) = (&pair[0], &pair[1]);
		let change = std::array::from_fn(|component| to.radiance[component] - from.radiance[component]);
		let _ = write!(
			code,
			"	if (age > {start}) {{
		color = {from} + {change} * clamp((age - {start}) * {inverse_span}, 0.0, 1.0);
	}}
",
			start = literal(from.age),
			from = vector(from.radiance),
			change = vector(change),
			inverse_span = literal(1.0 / (to.age - from.age)),
		);
	}
	code
}

fn fragment_program(shape: &Shape) -> String {
	let coverage = match shape {
		// Soft across the streak and solid along it.
		Shape::Streak { .. } => "1.0 - pipeline_input.uv.y * pipeline_input.uv.y",
		// A round sprite that fades to its edge.
		Shape::Billboard { .. } => "max(1.0 - dot(pipeline_input.uv, pipeline_input.uv), 0.0)",
	};
	format!(
		"// Generated from a `.particles` asset. The output is premultiplied with zero alpha, so particles add their light
// to the scene without hiding it or needing to be sorted.
main: fn (pipeline_input: interface {{ radiance: vec3f, uv: vec2f }}) -> output {{ color_attachment: vec4f }} {{
	let coverage: f32 = {coverage};
	let radiance: vec3f = pipeline_input.radiance * coverage;
	return {{ color_attachment: vec4f(radiance.x, radiance.y, radiance.z, 0.0) }};
}}
"
	)
}

/// Writes a float as a BESL literal, which needs a decimal point, no exponent, and no unary minus.
fn literal(value: f32) -> String {
	if value < 0.0 {
		format!("(0.0 - {:.7})", -value)
	} else {
		format!("{value:.7}")
	}
}

fn vector([x, y, z]: [f32; 3]) -> String {
	format!("vec3f({}, {}, {})", literal(x), literal(y), literal(z))
}
