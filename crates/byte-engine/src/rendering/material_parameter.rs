//! Messages that change dynamic material parameters from gameplay code.

use std::borrow::Cow;

use resource_management::bemx;

use crate::core::{factory::Handle, message::Message};

/// The `SetMaterialParameter` struct lets gameplay code animate a material parameter its `.bema` file declares
/// `Dynamic`, on one entity or on every entity drawn with a material.
///
/// Send it on [`World::material_parameters_channel`](crate::gameplay::World::material_parameters_channel); see
/// [Change a parameter at runtime](https://byte-engine.0x44491229.dev/docs/use/resources/author#change-a-parameter-at-runtime).
#[derive(Clone, Debug)]
pub struct SetMaterialParameter {
	pub(crate) target: MaterialParameterTarget,
	pub(crate) name: Cow<'static, str>,
	pub(crate) value: bemx::Value,
}

impl SetMaterialParameter {
	/// Sets the parameter `name` of the materials `entity` renders with to `value`, such as `0.5` or
	/// `[1.0, 0.0, 0.0, 1.0]`.
	pub fn new(entity: Handle, name: impl Into<Cow<'static, str>>, value: impl Into<bemx::Value>) -> Self {
		Self {
			target: MaterialParameterTarget::Entity(entity),
			name: name.into(),
			value: value.into(),
		}
	}

	/// Sets the parameter `name` of the material whose resource ID is `material`, such as `"solid.bema"`, to `value`
	/// for every entity that renders with it.
	pub fn material(
		material: impl Into<Cow<'static, str>>,
		name: impl Into<Cow<'static, str>>,
		value: impl Into<bemx::Value>,
	) -> Self {
		Self {
			target: MaterialParameterTarget::Material(material.into()),
			name: name.into(),
			value: value.into(),
		}
	}
}

impl Message for SetMaterialParameter {}

/// The `MaterialParameterTarget` enum names what a [`SetMaterialParameter`] changes.
#[derive(Clone, Debug)]
pub(crate) enum MaterialParameterTarget {
	/// Every primitive of one entity whose material declares the parameter.
	Entity(Handle),
	/// Every entity drawn with the material of this resource ID.
	Material(Cow<'static, str>),
}
