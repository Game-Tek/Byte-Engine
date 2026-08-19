use std::{alloc::Allocator, fmt};

use exr::prelude::f16;

use super::{
	handler::{AssetHandler, BakeContext, LoadErrors},
	ResourceId,
};
use crate::{
	resources::image::{Image, ImagePhotometry},
	types::{Formats, Gamma},
	ProcessedAsset,
};

/// Horizontal texel count for baked IES C-plane intensity maps.
pub const IES_INTENSITY_MAP_WIDTH: u32 = 721;

/// Vertical texel count for baked IES C-plane intensity maps.
pub const IES_INTENSITY_MAP_HEIGHT: u32 = 361;

const MAX_IES_SAMPLES: usize = 1_000_000;

const ANGLE_EPSILON_DEGREES: f32 = 0.001;

/// The `IESAssetHandler` struct bakes LM-63 photometric profiles into calibrated intensity-map images.
#[derive(Default)]

pub struct IESAssetHandler;

impl IESAssetHandler {
	/// Creates a handler for ASCII LM-63 IES photometric files.
	///
	/// Next, register this handler with [`crate::asset::manager::AssetManager::add_asset_handler`].
	pub fn new() -> Self {
		Self
	}
}

impl AssetHandler for IESAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type.eq_ignore_ascii_case("ies")
			|| r#type.eq_ignore_ascii_case("application/ies")
			|| r#type.eq_ignore_ascii_case("application/x-ies")
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		if let Some(resource_type) = context.resource_type(url) {
			if !self.can_handle(resource_type) {
				return Err(LoadErrors::UnsupportedType);
			}
		}

		let (source, _, resource_type) = context.resolve(url).await?;

		if !self.can_handle(&resource_type) {
			return Err(LoadErrors::UnsupportedType);
		}

		let profile = parse_ies(&source).map_err(|error| {
			context.error(format_args!("IES asset '{}': {error}", url.as_ref()));

			LoadErrors::FailedToProcess
		})?;

		let (data, intensity_scale_candela) = encode_intensity_map(&profile, context.allocator()).map_err(|error| {
			context.error(format_args!("IES asset '{}': {error}", url.as_ref()));

			LoadErrors::FailedToProcess
		})?;

		let image = Image {
			format: Formats::R16F,
			gamma: Gamma::Linear,
			extent: [IES_INTENSITY_MAP_WIDTH, IES_INTENSITY_MAP_HEIGHT, 1],
			mip_count: 1,
			ibl: None,
			photometry: Some(ImagePhotometry { intensity_scale_candela }),
		};

		context.store_primary(ProcessedAsset::new(url, image), &data)
	}
}

/// The `ParsedIes` struct preserves the calibrated C-plane table while an IES asset is baked.
struct ParsedIes {
	vertical_angles_degrees: Vec<f32>,
	horizontal_angles_degrees: Vec<f32>,
	candelas: Vec<f32>,
	candela_multiplier: f32,
	tilt: Option<TiltTable>,
	horizontal_symmetry: HorizontalSymmetry,
}

impl ParsedIes {
	/// Samples the profile's calibrated luminous intensity for one C-plane direction.
	fn sample_candela(&self, horizontal_angle_degrees: f32, vertical_angle_degrees: f32) -> f32 {
		let horizontal_angle_degrees = self.horizontal_symmetry.fold(horizontal_angle_degrees);

		let Some(vertical_intensity) = self.sample_horizontal_table(horizontal_angle_degrees, vertical_angle_degrees) else {
			return 0.0;
		};

		let tilt_factor = self
			.tilt
			.as_ref()
			.map(|tilt| tilt.factor_at(vertical_angle_degrees))
			.unwrap_or(1.0);

		vertical_intensity * tilt_factor * self.candela_multiplier
	}

	/// Samples the two bounding horizontal planes after sampling their vertical-angle tables.
	fn sample_horizontal_table(&self, horizontal_angle_degrees: f32, vertical_angle_degrees: f32) -> Option<f32> {
		let vertical_count = self.vertical_angles_degrees.len();

		let horizontal_index = self
			.horizontal_angles_degrees
			.partition_point(|angle| *angle <= horizontal_angle_degrees);

		match horizontal_index {
			0 => self.sample_vertical_table(0, vertical_angle_degrees),
			index if index == self.horizontal_angles_degrees.len() => {
				self.sample_vertical_table(index - 1, vertical_angle_degrees)
			}
			upper_index => {
				let lower_index = upper_index - 1;

				let lower_angle = self.horizontal_angles_degrees[lower_index];

				let upper_angle = self.horizontal_angles_degrees[upper_index];

				let factor = (horizontal_angle_degrees - lower_angle) / (upper_angle - lower_angle);

				let lower = self.sample_vertical_table(lower_index, vertical_angle_degrees)?;

				let upper = self.sample_vertical_table(upper_index, vertical_angle_degrees)?;

				debug_assert_eq!(self.candelas.len(), vertical_count * self.horizontal_angles_degrees.len());

				Some(lerp(lower, upper, factor))
			}
		}
	}

	/// Samples one authored horizontal plane and returns zero outside its measured vertical range.
	fn sample_vertical_table(&self, horizontal_index: usize, vertical_angle_degrees: f32) -> Option<f32> {
		let first = *self.vertical_angles_degrees.first()?;

		let last = *self.vertical_angles_degrees.last()?;

		if vertical_angle_degrees < first || vertical_angle_degrees > last {
			return Some(0.0);
		}

		let vertical_count = self.vertical_angles_degrees.len();

		let samples = &self.candelas[horizontal_index * vertical_count..][..vertical_count];

		let upper_index = self
			.vertical_angles_degrees
			.partition_point(|angle| *angle <= vertical_angle_degrees);

		match upper_index {
			0 => Some(samples[0]),
			index if index == vertical_count => Some(samples[vertical_count - 1]),
			upper_index => {
				let lower_index = upper_index - 1;

				let lower_angle = self.vertical_angles_degrees[lower_index];

				let upper_angle = self.vertical_angles_degrees[upper_index];

				let factor = (vertical_angle_degrees - lower_angle) / (upper_angle - lower_angle);

				Some(lerp(samples[lower_index], samples[upper_index], factor))
			}
		}
	}
}

/// The `HorizontalSymmetry` enum expands the standard C-plane angle ranges into a full revolution.
#[derive(Clone, Copy)]

enum HorizontalSymmetry {
	Rotational,
	Quadrantal,
	Bilateral,
	None,
}

impl HorizontalSymmetry {
	/// Folds a full-revolution horizontal angle into the authored profile range.
	fn fold(self, horizontal_angle_degrees: f32) -> f32 {
		let horizontal_angle_degrees = horizontal_angle_degrees.rem_euclid(360.0);

		match self {
			Self::Rotational => 0.0,
			Self::Quadrantal => {
				let angle = horizontal_angle_degrees.rem_euclid(180.0);

				if angle > 90.0 {
					180.0 - angle
				} else {
					angle
				}
			}
			Self::Bilateral => {
				if horizontal_angle_degrees > 180.0 {
					360.0 - horizontal_angle_degrees
				} else {
					horizontal_angle_degrees
				}
			}
			Self::None => horizontal_angle_degrees,
		}
	}
}

/// The `TiltTable` struct adjusts luminous intensity for a luminaire's included lamp-tilt data.
struct TiltTable {
	angles_degrees: Vec<f32>,
	factors: Vec<f32>,
}

impl TiltTable {
	/// Interpolates an included tilt factor and clamps at the authored table boundaries.
	fn factor_at(&self, angle_degrees: f32) -> f32 {
		let upper_index = self.angles_degrees.partition_point(|angle| *angle <= angle_degrees);

		match upper_index {
			0 => self.factors[0],
			index if index == self.angles_degrees.len() => self.factors[self.factors.len() - 1],
			upper_index => {
				let lower_index = upper_index - 1;

				let factor = (angle_degrees - self.angles_degrees[lower_index])
					/ (self.angles_degrees[upper_index] - self.angles_degrees[lower_index]);

				lerp(self.factors[lower_index], self.factors[upper_index], factor)
			}
		}
	}
}

/// Parses an ASCII LM-63 file, including optional in-file tilt data, into its photometric table.
fn parse_ies(data: &[u8]) -> Result<ParsedIes, IesError> {
	let source = std::str::from_utf8(data).map_err(|error| {
		IesError::new(
			"Invalid IES text",
			format_args!("the file is not ASCII-compatible UTF-8: {error}"),
		)
	})?;

	let (tilt_directive, photometric_data) = find_tilt_directive(source)?;

	let mut numbers = NumberReader::new(photometric_data);

	let tilt = parse_tilt(tilt_directive, &mut numbers)?;

	let lamp_count = numbers.integer("lamp count")?;

	if lamp_count < 0 {
		return Err(IesError::new("Invalid IES lamp count", "the lamp count is negative"));
	}

	let _lumens_per_lamp = numbers.finite("lumens per lamp")?;

	let candela_multiplier = numbers.positive("candela multiplier")?;

	let vertical_count = numbers.count("vertical-angle count")?;

	let horizontal_count = numbers.count("horizontal-angle count")?;

	let photometric_type = numbers.integer("photometric type")?;

	if photometric_type != 1 {
		return Err(IesError::new(
			"Unsupported IES photometric type",
			"only LM-63 Type C profiles can be represented by the renderer's spherical intensity map",
		));
	}

	let units_type = numbers.integer("units type")?;

	if units_type != 1 && units_type != 2 {
		return Err(IesError::new(
			"Invalid IES units type",
			"the units type is neither 1 (feet) nor 2 (meters)",
		));
	}

	for field in [
		"luminaire width",
		"luminaire length",
		"luminaire height",
		"ballast factor",
		"future-use factor",
		"input watts",
	] {
		let _ = numbers.finite(field)?;
	}

	let vertical_angles_degrees = parse_angles(&mut numbers, vertical_count, "vertical", 180.0)?;

	let horizontal_angles_degrees = parse_angles(&mut numbers, horizontal_count, "horizontal", 360.0)?;

	let horizontal_symmetry = horizontal_symmetry(&horizontal_angles_degrees)?;

	let sample_count = vertical_count.checked_mul(horizontal_count).ok_or_else(|| {
		IesError::new(
			"IES intensity table is too large",
			"the product of its vertical and horizontal sample counts overflows",
		)
	})?;

	if sample_count > MAX_IES_SAMPLES {
		return Err(IesError::new(
			"IES intensity table is too large",
			format_args!("it contains {sample_count} samples, exceeding the {MAX_IES_SAMPLES}-sample safety limit"),
		));
	}

	let mut candelas = Vec::new();

	candelas.try_reserve_exact(sample_count).map_err(|_| {
		IesError::new(
			"IES intensity table could not be allocated",
			"the process does not have enough memory for the declared samples",
		)
	})?;

	for _ in 0..sample_count {
		candelas.push(numbers.nonnegative("candela value")?);
	}

	Ok(ParsedIes {
		vertical_angles_degrees,
		horizontal_angles_degrees,
		candelas,
		candela_multiplier,
		tilt,
		horizontal_symmetry,
	})
}

/// Locates `TILT=` and returns its value together with the numeric LM-63 body that follows it.
fn find_tilt_directive(source: &str) -> Result<(&str, &str), IesError> {
	let mut consumed = 0;

	for line in source.split_inclusive('\n') {
		let trimmed = line.trim();

		if let Some((name, value)) = trimmed.split_once('=') {
			if name.trim().eq_ignore_ascii_case("TILT") {
				return Ok((value.trim(), &source[consumed + line.len()..]));
			}
		}

		consumed += line.len();
	}

	Err(IesError::new(
		"Missing IES TILT directive",
		"the file does not contain the required `TILT=NONE`, `TILT=INCLUDE`, or external tilt reference line",
	))
}

/// Parses no tilt data or the optional data embedded after `TILT=INCLUDE`.
fn parse_tilt(directive: &str, numbers: &mut NumberReader<'_>) -> Result<Option<TiltTable>, IesError> {
	if directive.eq_ignore_ascii_case("NONE") {
		return Ok(None);
	}

	if !directive.eq_ignore_ascii_case("INCLUDE") {
		return Err(IesError::new(
			"Unsupported external IES tilt table",
			"the profile references a separate tilt file, which is not available through this single-asset handler",
		));
	}

	let geometry = numbers.integer("tilt geometry")?;

	if !(1..=3).contains(&geometry) {
		return Err(IesError::new(
			"Invalid IES tilt geometry",
			"the included tilt geometry is not one of the LM-63 values 1, 2, or 3",
		));
	}

	let count = numbers.count("tilt-angle count")?;

	let angles_degrees = parse_angles(numbers, count, "tilt", 180.0)?;

	let mut factors = Vec::new();

	factors.try_reserve_exact(count).map_err(|_| {
		IesError::new(
			"IES tilt table could not be allocated",
			"the process does not have enough memory for the declared tilt samples",
		)
	})?;

	for _ in 0..count {
		factors.push(numbers.nonnegative("tilt factor")?);
	}

	Ok(Some(TiltTable { angles_degrees, factors }))
}

/// Parses one strictly ascending angular axis in degrees.
fn parse_angles(
	numbers: &mut NumberReader<'_>,
	count: usize,
	axis: &str,
	maximum_angle_degrees: f32,
) -> Result<Vec<f32>, IesError> {
	let mut angles = Vec::new();

	angles.try_reserve_exact(count).map_err(|_| {
		IesError::new(
			"IES angle table could not be allocated",
			"the process does not have enough memory for the declared angle samples",
		)
	})?;

	for index in 0..count {
		let angle = numbers.finite(&format!("{axis} angle"))?;

		if !(0.0..=maximum_angle_degrees).contains(&angle) {
			return Err(IesError::new(
				"Invalid IES angle",
				format_args!("{axis} angle {angle} is outside 0 through {maximum_angle_degrees} degrees"),
			));
		}

		if index > 0 && angle <= angles[index - 1] {
			return Err(IesError::new(
				"Invalid IES angle order",
				format_args!("the {axis} angles are not strictly ascending"),
			));
		}

		angles.push(angle);
	}

	Ok(angles)
}

/// Determines how a standard Type C horizontal table repeats over a full revolution.
fn horizontal_symmetry(angles_degrees: &[f32]) -> Result<HorizontalSymmetry, IesError> {
	let first = angles_degrees[0];

	if first.abs() > ANGLE_EPSILON_DEGREES {
		return Err(IesError::new(
			"Unsupported IES horizontal angle range",
			"Type C profiles must begin at 0 degrees so their symmetry can be expanded",
		));
	}

	if angles_degrees.len() == 1 {
		return Ok(HorizontalSymmetry::Rotational);
	}

	let last = *angles_degrees
		.last()
		.expect("non-empty IES angle tables are validated before symmetry detection");

	if (last - 90.0).abs() <= ANGLE_EPSILON_DEGREES {
		Ok(HorizontalSymmetry::Quadrantal)
	} else if (last - 180.0).abs() <= ANGLE_EPSILON_DEGREES {
		Ok(HorizontalSymmetry::Bilateral)
	} else if (last - 360.0).abs() <= ANGLE_EPSILON_DEGREES {
		Ok(HorizontalSymmetry::None)
	} else {
		Err(IesError::new(
			"Unsupported IES horizontal angle range",
			"Type C profiles must end at 90, 180, or 360 degrees",
		))
	}
}

/// Encodes the calibrated profile into a normalized half-float texture and returns its candela scale.
fn encode_intensity_map<'a>(
	profile: &ParsedIes,
	allocator: &'a dyn Allocator,
) -> Result<(Vec<u8, &'a dyn Allocator>, f32), IesError> {
	let mut maximum_candela = 0.0_f32;

	for y in 0..IES_INTENSITY_MAP_HEIGHT {
		let vertical_angle = texture_vertical_angle(y);

		for x in 0..IES_INTENSITY_MAP_WIDTH {
			maximum_candela = maximum_candela.max(profile.sample_candela(texture_horizontal_angle(x), vertical_angle));
		}
	}

	if !maximum_candela.is_finite() || maximum_candela <= 0.0 {
		return Err(IesError::new(
			"IES profile has no usable luminous intensity",
			"every calibrated candela value is zero or non-finite",
		));
	}

	let byte_count = (IES_INTENSITY_MAP_WIDTH as usize)
		.checked_mul(IES_INTENSITY_MAP_HEIGHT as usize)
		.and_then(|texel_count| texel_count.checked_mul(std::mem::size_of::<f16>()))
		.ok_or_else(|| {
			IesError::new(
				"IES intensity map is too large",
				"the configured texture dimensions overflow the output byte count",
			)
		})?;

	let mut data = Vec::new_in(allocator);

	data.try_reserve_exact(byte_count).map_err(|_| {
		IesError::new(
			"IES intensity map could not be allocated",
			"the bake allocator does not have enough memory for the output texture",
		)
	})?;

	data.resize(byte_count, 0);

	for y in 0..IES_INTENSITY_MAP_HEIGHT {
		let vertical_angle = texture_vertical_angle(y);

		for x in 0..IES_INTENSITY_MAP_WIDTH {
			let intensity =
				(profile.sample_candela(texture_horizontal_angle(x), vertical_angle) / maximum_candela).clamp(0.0, 1.0);

			let texel_index = (y as usize * IES_INTENSITY_MAP_WIDTH as usize + x as usize) * std::mem::size_of::<f16>();

			data[texel_index..texel_index + std::mem::size_of::<f16>()]
				.copy_from_slice(&f16::from_f32(intensity).to_le_bytes());
		}
	}

	Ok((data, maximum_candela))
}

/// Maps one horizontal texel to its full C-plane angle, including a duplicate seam texel at 360 degrees.
fn texture_horizontal_angle(x: u32) -> f32 {
	360.0 * x as f32 / (IES_INTENSITY_MAP_WIDTH - 1) as f32
}

/// Maps one vertical texel to its full Type C vertical angle range.
fn texture_vertical_angle(y: u32) -> f32 {
	180.0 * y as f32 / (IES_INTENSITY_MAP_HEIGHT - 1) as f32
}

/// Reads LM-63 whitespace-delimited numeric fields while retaining useful field names for errors.
struct NumberReader<'a> {
	tokens: std::str::SplitWhitespace<'a>,
}

impl<'a> NumberReader<'a> {
	/// Creates a reader for the numeric body after the `TILT=` directive.
	fn new(source: &'a str) -> Self {
		Self {
			tokens: source.split_whitespace(),
		}
	}

	/// Reads one required source token.
	fn token(&mut self, field: &str) -> Result<&'a str, IesError> {
		self.tokens.next().ok_or_else(|| {
			IesError::new(
				"Incomplete IES photometric data",
				format_args!("the required {field} field is missing"),
			)
		})
	}

	/// Reads a finite floating-point source field.
	fn finite(&mut self, field: &str) -> Result<f32, IesError> {
		let token = self.token(field)?;

		let value = token.parse::<f32>().map_err(|_| {
			IesError::new(
				"Invalid IES numeric field",
				format_args!("{field} is not a floating-point number"),
			)
		})?;

		if !value.is_finite() {
			return Err(IesError::new(
				"Invalid IES numeric field",
				format_args!("{field} is not finite"),
			));
		}

		Ok(value)
	}

	/// Reads a nonnegative floating-point source field.
	fn nonnegative(&mut self, field: &str) -> Result<f32, IesError> {
		let value = self.finite(field)?;

		if value < 0.0 {
			return Err(IesError::new(
				"Invalid IES numeric field",
				format_args!("{field} is negative"),
			));
		}

		Ok(value)
	}

	/// Reads a positive floating-point source field.
	fn positive(&mut self, field: &str) -> Result<f32, IesError> {
		let value = self.finite(field)?;

		if value <= 0.0 {
			return Err(IesError::new(
				"Invalid IES numeric field",
				format_args!("{field} is not positive"),
			));
		}

		Ok(value)
	}

	/// Reads a signed integer source field.
	fn integer(&mut self, field: &str) -> Result<i32, IesError> {
		let token = self.token(field)?;

		token
			.parse::<i32>()
			.map_err(|_| IesError::new("Invalid IES integer field", format_args!("{field} is not an integer")))
	}

	/// Reads a bounded positive sample count.
	fn count(&mut self, field: &str) -> Result<usize, IesError> {
		let token = self.token(field)?;

		let count = token
			.parse::<usize>()
			.map_err(|_| IesError::new("Invalid IES sample count", format_args!("{field} is not a positive integer")))?;

		if count == 0 || count > MAX_IES_SAMPLES {
			return Err(IesError::new(
				"Invalid IES sample count",
				format_args!("{field} is outside 1 through {MAX_IES_SAMPLES}"),
			));
		}

		Ok(count)
	}
}

/// The `IesError` struct explains why an LM-63 source cannot produce a calibrated intensity map.
#[derive(Debug)]

struct IesError(String);

impl IesError {
	/// Builds an error with a short problem statement and its most likely cause.
	fn new(message: &str, cause: impl fmt::Display) -> Self {
		Self(format!("{message}. The most likely cause is {cause}."))
	}
}

impl fmt::Display for IesError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.0.fmt(formatter)
	}
}

fn lerp(lower: f32, upper: f32, factor: f32) -> f32 {
	lower + (upper - lower) * factor
}

#[cfg(test)]

mod tests {

	use std::sync::Arc;

	use exr::prelude::f16;

	use super::{parse_ies, IESAssetHandler, IES_INTENSITY_MAP_HEIGHT, IES_INTENSITY_MAP_WIDTH};
	use crate::{
		asset::{manager::AssetManager, storage_backend::tests::TestStorageBackend, ResourceId},
		r#async,
		resource::{
			storage_backend::tests::TestStorageBackend as TestResourceStorage, ReadStorageBackend as _, ReadTargetsMut,
		},
		resources::image::Image,
		types::{Formats, Gamma},
		ResourceManager,
	};

	const QUADRANT_PROFILE: &[u8] = br#"IESNA:LM-63-2002
[TEST] Quadrantal test profile
TILT=NONE
1 1000 2 3 3 1 2 0 0 0 1 1 10
0 90 180
0 45 90
10 20 30
40 50 60
70 80 90
"#;

	#[test]
	fn accepts_ies_extensions_and_mime_types_case_insensitively() {
		let handler = IESAssetHandler::new();

		assert!(crate::AssetHandler::can_handle(&handler, "ies"));
		assert!(crate::AssetHandler::can_handle(&handler, "IES"));
		assert!(crate::AssetHandler::can_handle(&handler, "application/ies"));
		assert!(crate::AssetHandler::can_handle(&handler, "application/x-ies"));
		assert!(!crate::AssetHandler::can_handle(&handler, "exr"));
	}

	#[test]
	fn parses_multiplier_and_expands_quadrantal_symmetry() {
		let profile = parse_ies(QUADRANT_PROFILE).expect("valid Type C IES profile");

		assert_eq!(profile.sample_candela(0.0, 90.0), 40.0);
		assert_eq!(profile.sample_candela(45.0, 90.0), 100.0);
		assert_eq!(profile.sample_candela(90.0, 90.0), 160.0);
		assert_eq!(profile.sample_candela(135.0, 90.0), 100.0);
		assert_eq!(profile.sample_candela(270.0, 90.0), 160.0);
		assert_eq!(profile.sample_candela(45.0, 181.0), 0.0);
	}

	#[test]
	fn applies_included_tilt_factors() {
		let profile = parse_ies(
			br#"IESNA:LM-63-2002
TILT=INCLUDE
1
3
0 90 180
1 0.5 1
1 1000 1 3 1 1 2 0 0 0 1 1 10
0 90 180
0
100 100 100
"#,
		)
		.expect("valid IES profile with included tilt data");

		assert_eq!(profile.sample_candela(0.0, 0.0), 100.0);
		assert_eq!(profile.sample_candela(0.0, 90.0), 50.0);
		assert_eq!(profile.sample_candela(0.0, 180.0), 100.0);
	}

	#[test]
	fn rejects_non_c_photometry_with_an_actionable_error() {
		let unsupported = br#"IESNA:LM-63-2002
TILT=NONE
1 1000 1 3 1 2 2 0 0 0 1 1 10
0 90 180
0
100 100 100
"#;

		let error = match parse_ies(unsupported) {
			Err(error) => error,
			Ok(_) => panic!("Type B is not a spherical C-plane profile"),
		};

		assert!(error.to_string().starts_with("Unsupported IES photometric type."));
		assert!(error.to_string().contains("only LM-63 Type C profiles"));
	}

	#[cfg(debug_assertions)]
	#[r#async::test]
	async fn resource_manager_lazily_bakes_and_loads_ies_images() {
		let source_storage = TestStorageBackend::new();

		source_storage.add_file("lights/quadrant.ies", QUADRANT_PROFILE);

		let resource_storage = Arc::new(TestResourceStorage::new());

		let mut asset_manager = AssetManager::new_shared(source_storage, resource_storage.clone());

		asset_manager.add_asset_handler(IESAssetHandler::new());

		let resource_manager = ResourceManager::new_shared(resource_storage);

		resource_manager.set_asset_manager(asset_manager);

		let mut reference = resource_manager
			.request::<Image>("lights/quadrant.ies")
			.await
			.expect("the IES source must bake lazily as an image resource");

		assert_eq!(reference.resource().format, Formats::R16F);
		assert_eq!(reference.resource().gamma, Gamma::Linear);
		assert_eq!(
			reference.resource().extent,
			[IES_INTENSITY_MAP_WIDTH, IES_INTENSITY_MAP_HEIGHT, 1]
		);
		assert!(reference.resource().photometry.is_some());

		let loaded = reference
			.load(ReadTargetsMut::backing_storage())
			.await
			.expect("the lazily baked IES intensity texels must load");

		assert_eq!(
			loaded.buffer().map(<[u8]>::len),
			Some((IES_INTENSITY_MAP_WIDTH * IES_INTENSITY_MAP_HEIGHT * 2) as usize)
		);
	}

	#[r#async::test]
	async fn bakes_a_calibrated_r16f_intensity_map() {
		let source_storage = TestStorageBackend::new();

		source_storage.add_file("lights/quadrant.ies", QUADRANT_PROFILE);

		let resource_storage = TestResourceStorage::new();

		let mut asset_manager = AssetManager::new(source_storage, resource_storage.clone());

		asset_manager.add_asset_handler(IESAssetHandler::new());

		asset_manager
			.bake("lights/quadrant.ies")
			.await
			.expect("the registered IES handler must bake the profile");

		let (stored, _) = resource_storage
			.read(ResourceId::new("lights/quadrant.ies"))
			.await
			.expect("the baked IES image must be stored");

		let image: Image = crate::from_slice(stored.resource()).expect("the IES image metadata must deserialize");

		assert_eq!(image.format, Formats::R16F);
		assert_eq!(image.gamma, Gamma::Linear);
		assert_eq!(image.extent, [IES_INTENSITY_MAP_WIDTH, IES_INTENSITY_MAP_HEIGHT, 1]);
		assert_eq!(image.mip_count, 1);

		let intensity_scale_candela = image
			.photometry
			.expect("IES images include their candela scale")
			.intensity_scale_candela;

		assert!(
			(intensity_scale_candela - 180.0).abs() < 0.001,
			"expected 180 cd but received {intensity_scale_candela} cd"
		);

		let data = resource_storage
			.get_resource_data_by_name(ResourceId::new("lights/quadrant.ies"))
			.expect("the baked IES intensity texels must exist");

		let first_texel = f16::from_le_bytes(data[..2].try_into().expect("an R16F texel is two bytes")).to_f32();

		assert!((first_texel - (20.0 / 180.0)).abs() < 0.001);
	}
}
