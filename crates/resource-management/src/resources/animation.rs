use crate::{resource, solver::SolveErrors, Model, Resource, Solver};

/// The `Animation` resource represents a collection of animation data that can be applied to nodes.
/// It contains samplers (interpolation functions) and channels (targets for animation).
#[derive(Debug, serde::Serialize)]
pub struct Animation {
    pub name: Option<String>,
    pub samplers: Vec<AnimationSampler>,
    pub channels: Vec<AnimationChannel>,
    pub duration: f32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct AnimationModel {
    pub name: Option<String>,
    pub samplers: Vec<AnimationSampler>,
    pub channels: Vec<AnimationChannel>,
    pub duration: f32,
}

impl Resource for Animation {
    fn get_class(&self) -> &'static str {
        "Animation"
    }
    type Model = AnimationModel;
}

impl Model for AnimationModel {
    fn get_class() -> &'static str {
        "Animation"
    }
}

impl<'de> Solver<'de, Animation> for AnimationModel {
    fn solve(
        self,
        _storage_backend: &dyn resource::ReadStorageBackend,
    ) -> Result<Animation, SolveErrors> {
        let AnimationModel {
            name,
            samplers,
            channels,
            duration,
        } = self;

        Ok(Animation {
            name,
            samplers,
            channels,
            duration,
        })
    }
}

/// The `AnimationSampler` defines how keyframes are interpolated.
/// Input: times (f32 array)
/// Output: values (depends on target path)
/// Interpolation: LINEAR, STEP, or CUBICSPLINE
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct AnimationSampler {
    pub interpolation: Interpolation,
    pub input_times: Vec<f32>,
    pub output_values: SamplerOutput,
}

/// The `Interpolation` enum defines how keyframes are interpolated.
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    Linear,
    Step,
    CubicSpline,
}

impl From<gltf::animation::Interpolation> for Interpolation {
    fn from(interp: gltf::animation::Interpolation) -> Self {
        match interp {
            gltf::animation::Interpolation::Linear => Interpolation::Linear,
            gltf::animation::Interpolation::Step => Interpolation::Step,
            gltf::animation::Interpolation::CubicSpline => Interpolation::CubicSpline,
        }
    }
}

/// The `SamplerOutput` represents the output values of an animation sampler.
/// The type depends on what is being animated (translation, rotation, scale, or weights).
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub enum SamplerOutput {
    Translation(Vec<[f32; 3]>),
    Rotation(Vec<[f32; 4]>),
    Scale(Vec<[f32; 3]>),
    Weights(Vec<f32>),
}

/// The `AnimationChannel` links a sampler to a target node/path.
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct AnimationChannel {
    pub sampler_index: usize,
    pub target_node: usize,
    pub target_path: AnimationPath,
}

/// The `AnimationPath` specifies which property of the node is being animated.
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum AnimationPath {
    Translation,
    Rotation,
    Scale,
    Weights,
}

impl From<gltf::animation::Property> for AnimationPath {
    fn from(prop: gltf::animation::Property) -> Self {
        match prop {
            gltf::animation::Property::Translation => AnimationPath::Translation,
            gltf::animation::Property::Rotation => AnimationPath::Rotation,
            gltf::animation::Property::Scale => AnimationPath::Scale,
            gltf::animation::Property::MorphTargetWeights => AnimationPath::Weights,
        }
    }
}
