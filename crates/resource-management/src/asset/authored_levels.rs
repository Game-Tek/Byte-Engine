//! Find the files that hold a model's authored detail levels from the `lods.authored` pattern of its BEAD settings.

use std::fmt::Write as _;

use super::BEADType;

/// The `AuthoredLevelPattern` struct lets a directory of models name, once, which files hold each model's coarser
/// authored detail levels, such as `rock_LOD1.fbx` and `rock_LOD2.fbx` for `rock_LOD0.fbx`.
///
/// Read it from a model's BEAD settings with [`authored_level_pattern`], then list a model's level files with
/// [`Self::coarser_level_ids`] and read them as sources of the model's bake.
#[derive(Debug, PartialEq)]
pub(crate) struct AuthoredLevelPattern {
	parts: Vec<PatternPart>,
}

#[derive(Debug, PartialEq)]
enum PatternPart {
	Text(String),
	/// `{name}`: any non-empty text, the same in every file of one model's levels.
	Any(String),
	/// `{level}`: the decimal detail level number.
	Level,
}

impl AuthoredLevelPattern {
	/// Parses `{level}`, `{name}` placeholders and literal text. Exactly one `{level}` is required.
	pub(crate) fn parse(pattern: &str) -> Result<Self, String> {
		let mut parts = Vec::new();
		let mut rest = pattern;
		while !rest.is_empty() {
			match rest.find('{') {
				Some(0) => {
					let end = rest
						.find('}')
						.ok_or_else(|| format!("`lods.authored` '{pattern}' has a `{{` without a closing `}}`"))?;
					let name = &rest[1..end];
					if name.is_empty()
						|| !name
							.chars()
							.all(|character| character.is_ascii_alphanumeric() || character == '_')
					{
						return Err(format!(
							"`lods.authored` '{pattern}' has the placeholder `{{{name}}}`, but a placeholder name holds only letters, digits, and underscores"
						));
					}
					if matches!(parts.last(), Some(PatternPart::Any(_) | PatternPart::Level)) {
						return Err(format!(
							"`lods.authored` '{pattern}' puts two placeholders next to each other, so the file names can't say where one ends"
						));
					}
					parts.push(if name == "level" {
						PatternPart::Level
					} else {
						PatternPart::Any(name.to_string())
					});
					rest = &rest[end + 1..];
				}
				Some(start) => {
					parts.push(PatternPart::Text(rest[..start].to_string()));
					rest = &rest[start..];
				}
				None => {
					parts.push(PatternPart::Text(rest.to_string()));
					rest = "";
				}
			}
		}
		if parts.iter().filter(|part| **part == PatternPart::Level).count() != 1 {
			return Err(format!("`lods.authored` '{pattern}' must hold `{{level}}` exactly once"));
		}
		Ok(Self { parts })
	}

	/// Returns the asset IDs of the files that would hold `model_id`'s coarser levels, finest first: the model's
	/// directory and extension, with the pattern's level one, two, and more above the model's own.
	///
	/// Returns `None` when the model's file name, without its extension, doesn't match the pattern. The caller stops
	/// at the first file that doesn't exist.
	pub(crate) fn coarser_level_ids(&self, model_id: &str) -> Option<impl Iterator<Item = String> + '_> {
		let (directory, file) = model_id
			.rsplit_once('/')
			.map_or(("", model_id), |(directory, file)| (directory, file));
		let (stem, extension) = file
			.rsplit_once('.')
			.map_or((file, None), |(stem, extension)| (stem, Some(extension)));
		let mut captures = Vec::new();
		let mut level = None;
		if !match_parts(&self.parts, stem, &mut captures, &mut level) {
			return None;
		}
		let level_text = level?;
		let model_level = level_text.parse::<u32>().ok()?;
		let width = level_text.len();
		let (directory, extension) = (directory.to_string(), extension.map(str::to_string));
		let captures: Vec<(String, String)> = captures
			.into_iter()
			.map(|(name, value)| (name.to_string(), value.to_string()))
			.collect();
		Some((model_level + 1..).map(move |level| {
			let mut id = String::new();
			if !directory.is_empty() {
				id.push_str(&directory);
				id.push('/');
			}
			for part in &self.parts {
				match part {
					PatternPart::Text(text) => id.push_str(text),
					PatternPart::Any(name) => {
						let value = captures.iter().find(|(captured, _)| captured == name).map(|(_, value)| value);
						id.push_str(value.map_or("", String::as_str));
					}
					// Keeps the model's zero padding, so `LOD01` is followed by `LOD02`.
					PatternPart::Level => {
						let _ = write!(id, "{level:0width$}");
					}
				}
			}
			if let Some(extension) = &extension {
				id.push('.');
				id.push_str(extension);
			}
			id
		}))
	}
}

/// Matches `text` against `parts` as a whole, trying every split of each placeholder, and records what each
/// placeholder matched. A placeholder named twice must match the same text both times.
fn match_parts<'p, 't>(
	parts: &'p [PatternPart],
	text: &'t str,
	captures: &mut Vec<(&'p str, &'t str)>,
	level: &mut Option<&'t str>,
) -> bool {
	let Some((part, rest)) = parts.split_first() else {
		return text.is_empty();
	};
	match part {
		PatternPart::Text(literal) => text
			.strip_prefix(literal.as_str())
			.is_some_and(|remaining| match_parts(rest, remaining, captures, level)),
		PatternPart::Level => {
			let digits = text.bytes().take_while(u8::is_ascii_digit).count();
			(1..=digits).rev().any(|end| {
				*level = Some(&text[..end]);
				match_parts(rest, &text[end..], captures, level)
			})
		}
		PatternPart::Any(name) => {
			let ends = text.char_indices().map(|(index, _)| index).skip(1).chain([text.len()]);
			for end in ends.filter(|&end| end > 0) {
				let value = &text[..end];
				match captures.iter().find(|(captured, _)| captured == name) {
					Some(&(_, earlier)) => {
						if earlier == value && match_parts(rest, &text[end..], captures, level) {
							return true;
						}
					}
					None => {
						captures.push((name.as_str(), value));
						if match_parts(rest, &text[end..], captures, level) {
							return true;
						}
						captures.pop();
					}
				}
			}
			false
		}
	}
}

/// Reads the `lods.authored` pattern of a model's BEAD settings, or `None` when it sets none.
pub(crate) fn authored_level_pattern(spec: Option<&BEADType>) -> Result<Option<AuthoredLevelPattern>, String> {
	match spec.and_then(|spec| spec.get("lods")).and_then(|lods| lods.get("authored")) {
		None => Ok(None),
		Some(value) => {
			let pattern = value
				.as_str()
				.ok_or_else(|| "`lods.authored` must be a string such as \"{name}_LOD{level}\"".to_string())?;
			AuthoredLevelPattern::parse(pattern).map(Some)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::AuthoredLevelPattern;

	fn levels(pattern: &str, model: &str) -> Option<Vec<String>> {
		let pattern = AuthoredLevelPattern::parse(pattern).expect("pattern should parse");
		pattern.coarser_level_ids(model).map(|ids| ids.take(3).collect())
	}

	#[test]
	fn a_model_whose_name_matches_lists_its_coarser_level_files_in_its_directory() {
		assert_eq!(
			levels("{name}_LOD{level}", "plants/fern/fern_VarA_LOD0.fbx").unwrap(),
			[
				"plants/fern/fern_VarA_LOD1.fbx",
				"plants/fern/fern_VarA_LOD2.fbx",
				"plants/fern/fern_VarA_LOD3.fbx"
			]
		);
		assert_eq!(
			levels("{name}_LOD{level}", "fern_LOD1.fbx").unwrap(),
			["fern_LOD2.fbx", "fern_LOD3.fbx", "fern_LOD4.fbx"]
		);
		assert_eq!(
			levels("{name}_lod{level}", "rock_lod08.fbx").unwrap()[..2],
			["rock_lod09.fbx", "rock_lod10.fbx"]
		);
	}

	#[test]
	fn a_model_whose_name_does_not_match_has_no_authored_levels() {
		assert_eq!(levels("{name}_LOD{level}", "rocks/xbsgdeh/xbsgdeh.fbx"), None);
		assert_eq!(levels("{name}_LOD{level}", "rocks/_LOD0.fbx"), None);
	}

	#[test]
	fn patterns_need_one_level_and_separable_placeholders() {
		for pattern in [
			"{name}_LOD",
			"{level}_{level}",
			"{name}{level}",
			"{name_LOD{level}",
			"{na-me}_{level}",
		] {
			assert!(
				AuthoredLevelPattern::parse(pattern).is_err(),
				"'{pattern}' should be rejected"
			);
		}
	}
}
