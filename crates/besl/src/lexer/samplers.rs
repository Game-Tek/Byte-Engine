//! Links `sampler` declarations to the texture descriptors that name them, and checks the reads a reducing sampler
//! allows. See the [sampler specification](/docs/reference/besl/interface#samplers).

use super::*;
use crate::sampler::{Address, Anisotropy, Filter, Reduction, SamplerState};

const SAMPLERS_DOCS_PATH: &str = "reference/besl/interface#samplers";

/// Gives a texture descriptor the state of the sampler it names, after checking that the descriptor can take it.
///
/// `declared` holds the samplers declared before the descriptor, so a descriptor can't name a later one.
pub(super) fn attach_sampler(
	descriptor: &str,
	r#type: BindingTypes,
	sampler: Option<&str>,
	declared: &[(String, SamplerState)],
) -> Result<BindingTypes, LexError> {
	let Some(sampler) = sampler else {
		return Ok(r#type);
	};
	let Some((_, state)) = declared.iter().find(|(name, _)| name == sampler) else {
		return Err(LexError::invalid(format!(
			"Descriptor `{descriptor}` names sampler `{sampler}`, which isn't declared before it. The most likely cause is a misspelled sampler name or a sampler declared after the descriptor. Declare `{sampler}: sampler<{{ ... }}>;` above the descriptor. See {}.",
			crate::online_docs_url(SAMPLERS_DOCS_PATH)
		)));
	};
	let BindingTypes::CombinedImageSampler { format, .. } = r#type else {
		return Err(LexError::invalid(format!(
			"Descriptor `{descriptor}` names sampler `{sampler}`, but only texture descriptors take a sampler. The most likely cause is a `sampler` property on a buffer, storage image, or acceleration structure. Delete the property. See {}.",
			crate::online_docs_url(SAMPLERS_DOCS_PATH)
		)));
	};
	if state.anisotropy != Anisotropy::Off
		&& (state.filter != Filter::Linear || state.mip_filter != Filter::Linear || state.reduces())
	{
		return Err(LexError::invalid(format!(
			"Sampler `{sampler}` filters anisotropically but doesn't filter linearly and average. The most likely cause is a `nearest` filter or a min or max reduction on an anisotropic sampler. Remove the `anisotropy` property, or the `filter`, `mip_filter`, and `reduction` properties. See {}.",
			crate::online_docs_url(SAMPLERS_DOCS_PATH)
		)));
	}
	if state.reduces() {
		if state.mip_filter != Filter::Nearest {
			return Err(LexError::invalid(format!(
				"Sampler `{sampler}` reduces with `{}` but blends levels. The most likely cause is an omitted `mip_filter`, which defaults to `linear`. Add `mip_filter: nearest` to the sampler. See {}.",
				state.reduction.token(),
				crate::online_docs_url(SAMPLERS_DOCS_PATH)
			)));
		}
		if state.address != Address::Clamp {
			return Err(LexError::invalid(format!(
				"Sampler `{sampler}` reduces with `{}` but doesn't clamp coordinates. The most likely cause is a `repeat` or `mirror` address on a min or max sampler. Remove the `address` property, or set it to `clamp`. See {}.",
				state.reduction.token(),
				crate::online_docs_url(SAMPLERS_DOCS_PATH)
			)));
		}
		// An empty format is a 2D texture.
		if !format.is_empty() && format != "ArrayTexture2D" {
			return Err(LexError::invalid(format!(
				"Descriptor `{descriptor}` is a {format}, but its sampler `{sampler}` reduces with `{}`. The most likely cause is a reducing sampler on a texture that isn't 2D. Use a `Texture2D` or `Texture2DArray` descriptor, or a sampler with `reduction: average`. See {}.",
				state.reduction.token(),
				crate::online_docs_url(SAMPLERS_DOCS_PATH)
			)));
		}
	}
	Ok(BindingTypes::CombinedImageSampler {
		format,
		sampler: Some(*state),
	})
}

/// Checks that a texture intrinsic reads a texture the way its sampler allows: a reducing sampler takes only
/// explicit-LOD reads, and a downsample intrinsic needs the reduction it names.
pub(super) fn validate_sampled_read(intrinsic: &str, texture: &NodeReference) -> Result<(), LexError> {
	let Some(state) = binding_sampler(texture) else {
		return Ok(());
	};
	let required = match intrinsic {
		"downsample_min" => Reduction::Min,
		"downsample_max" => Reduction::Max,
		"sample" if state.reduces() => {
			return Err(LexError::invalid(format!(
				"`sample` reads a texture whose sampler reduces with `{}`. The most likely cause is a reducing sampler on a texture read with implicit derivatives. Read it with `texture_lod`, `downsample_min`, or `downsample_max`. See {}.",
				state.reduction.token(),
				crate::online_docs_url(SAMPLERS_DOCS_PATH)
			)));
		}
		_ => return Ok(()),
	};
	if state.reduction != required {
		return Err(LexError::invalid(format!(
			"`{intrinsic}` reads a texture whose sampler reduces with `{}`. The most likely cause is a downsample call that doesn't match the sampler. Call the downsample intrinsic that matches the sampler, or set the sampler's `reduction` to `{}`. See {}.",
			state.reduction.token(),
			required.token(),
			crate::online_docs_url(SAMPLERS_DOCS_PATH)
		)));
	}
	Ok(())
}

/// Checks that a function call doesn't take a texture whose sampler reduces. A function reads its texture parameters
/// without knowing their sampler, so it couldn't reduce where the platform doesn't.
pub(super) fn validate_function_argument(function: &str, argument: &NodeReference) -> Result<(), LexError> {
	match binding_sampler(argument) {
		Some(state) if state.reduces() => Err(LexError::invalid(format!(
			"Function `{function}` takes a texture whose sampler reduces with `{}`. The most likely cause is a helper function that reads a min or max texture through a parameter. Read the texture in the function that calls `{function}`, and pass the values it reads. See {}.",
			state.reduction.token(),
			crate::online_docs_url(SAMPLERS_DOCS_PATH)
		))),
		_ => Ok(()),
	}
}

/// Returns the state of the sampler a texture expression reads through, when its descriptor declares one.
///
/// Shader generators and the VM call it on a texture intrinsic's first argument, such as `occlusion_pyramid` or an
/// indexed layer `sprites[layer]`, to lower the read with the declared state.
pub fn binding_sampler(texture: &NodeReference) -> Option<SamplerState> {
	let borrowed = texture.borrow();
	match borrowed.node() {
		Nodes::Binding {
			r#type: BindingTypes::CombinedImageSampler { sampler, .. },
			..
		} => *sampler,
		Nodes::Expression(Expressions::Member { source, .. }) => binding_sampler(source),
		// An indexed texture, `textures[i]` or `layers[i]`, reads through the sampler of the indexed descriptor.
		Nodes::Expression(Expressions::Accessor { left, .. }) => binding_sampler(left),
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => binding_sampler(&elements[0]),
		_ => None,
	}
}
