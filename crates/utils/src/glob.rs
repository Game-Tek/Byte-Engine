//! Match IDs against `*` and `?` patterns, for configuration that selects assets and resources by ID.

/// The `Glob` struct lets configuration select IDs by pattern, such as the assets a project never bakes.
///
/// A pattern matches the whole ID, ignoring ASCII case: `*` matches any run of characters, including `/` and `#`,
/// and `?` matches one character. Build one with [`Glob::new`] once and call [`Glob::matches`] for each ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glob {
	/// The pattern in lowercase, one entry per character.
	pattern: Vec<char>,
}

impl Glob {
	/// Creates a glob from pattern text.
	pub fn new(pattern: &str) -> Glob {
		Glob {
			pattern: pattern.chars().map(|character| character.to_ascii_lowercase()).collect(),
		}
	}

	/// Returns whether the pattern matches all of `id`, ignoring ASCII case.
	pub fn matches(&self, id: &str) -> bool {
		let pattern = &self.pattern;
		// Iterative wildcard matching over byte offsets: on a mismatch, retry from the last `*` with it covering one more
		// character.
		let (mut pattern_at, mut id_at) = (0, 0);
		let mut retry: Option<(usize, usize)> = None;
		while let Some(character) = id[id_at..].chars().next() {
			match pattern.get(pattern_at) {
				Some('*') => {
					retry = Some((pattern_at, id_at));
					pattern_at += 1;
				}
				Some(&expected) if expected == '?' || expected == character.to_ascii_lowercase() => {
					pattern_at += 1;
					id_at += character.len_utf8();
				}
				_ => match retry {
					Some((star, covered)) => {
						// `covered` is before the end of the ID, because the loop only records offsets it hasn't passed.
						let covered = covered + id[covered..].chars().next().map_or(0, char::len_utf8);
						pattern_at = star + 1;
						id_at = covered;
						retry = Some((star, covered));
					}
					None => return false,
				},
			}
		}
		pattern[pattern_at..].iter().all(|&character| character == '*')
	}
}

#[cfg(test)]
mod tests {
	use super::Glob;

	#[test]
	fn a_glob_matches_the_whole_id_ignoring_case() {
		for (pattern, id, expected) in [
			("textures/wip/*", "textures/wip/rock.png", true),
			("textures/wip/*", "textures/wip/deep/rock.png", true),
			("TEXTURES/*.PNG", "textures/rock.png", true),
			("*.png", "rock.png.bead", false),
			("rock?.png", "rock1.png", true),
			("rock?.png", "rock.png", false),
			("scene.glb*", "scene.glb#images/0", true),
			("scene", "scene.glb", false),
			("*", "", true),
			("a*b*c", "axxbyyc", true),
			("a*b*c", "axxbyy", false),
		] {
			assert_eq!(Glob::new(pattern).matches(id), expected, "'{pattern}' against '{id}'");
		}
	}
}
