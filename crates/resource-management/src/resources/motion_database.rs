//! Baked motion databases: the frames of a set of animation clips, with the feature vectors a matched animation
//! state searches.
//!
//! Author a `.motion.bead` file that names the clips and the features, as the
//! [asset formats reference](/docs/reference/assets#motion-databases) specifies. Baking it stores one
//! [`MotionDatabase`], which `byte_engine::animation` loads into its clip pool beside the clips the database names.

/// The `MotionDatabase` struct lets a matched animation state find the frame of its clips that best continues the
/// character's pose and requested motion, without sampling any clip to search.
///
/// The bake samples every clip at the search's [`MotionSearch::sample_rate`] and stores each frame's normalized feature
/// vector in [`Self::features`] and its ground root in [`Self::ground`]. Frames are numbered across the database: the
/// frames of the first clip, then the next clip's, each clip's by time. Load the clips themselves by the IDs in
/// [`Self::clips`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct MotionDatabase {
	/// The clips, in database order.
	pub clips: Vec<MotionClip>,
	/// Every frame's normalized feature vector, frame after frame, [`MotionSearch::dimensions`] values each.
	pub features: Vec<f32>,
	/// Every frame's smoothed ground root: its X and Z position and its yaw about +Y, in the clip's object space.
	pub ground: Vec<[f32; 3]>,
	/// How a matched state measures, searches, and plays these frames.
	pub search: MotionSearch,
}

impl MotionDatabase {
	/// Returns how many frames the database holds across its clips.
	pub fn frame_count(&self) -> usize {
		self.ground.len()
	}
}

/// The `MotionSearch` struct is what a matched state needs from its [`MotionDatabase`] besides the frames: how to
/// build a query that compares with them, and how to search and play them.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct MotionSearch {
	/// The frames per second the clips are sampled at.
	pub sample_rate: f32,
	/// The name of the node the ground root projects.
	pub root: String,
	/// The nodes whose position or velocity are features, in ascending name order.
	pub nodes: Vec<MotionNode>,
	/// The times ahead of a frame, in seconds, that the trajectory features sample. Strictly ascending.
	pub trajectory_times: Vec<f32>,
	/// Whether the trajectory positions are features.
	pub trajectory_position: bool,
	/// Whether the trajectory directions are features.
	pub trajectory_direction: bool,
	/// Each feature dimension's mean over the pickable frames, which a query subtracts before it scales.
	pub mean: Vec<f32>,
	/// Each feature dimension's weight divided by its group's deviation, which a query multiplies by.
	pub scale: Vec<f32>,
	/// Seconds between searches while a matched state plays.
	pub interval: f32,
	/// Seconds from the playing time within which frames of the playing clip aren't candidates.
	pub exclusion: f32,
	/// Seconds a switch to a found frame inertializes over.
	pub blend: f32,
	/// The halflife, in seconds, of the spring that predicts the trajectory's position.
	pub velocity_halflife: f32,
	/// The halflife, in seconds, of the spring that predicts the trajectory's direction.
	pub facing_halflife: f32,
}

impl MotionSearch {
	/// Returns how many values each frame's feature vector holds.
	pub fn dimensions(&self) -> usize {
		self.mean.len()
	}

	/// Returns how many feature dimensions the node features take, which come before the trajectory's.
	pub fn node_dimensions(&self) -> usize {
		self.nodes
			.iter()
			.map(|node| 3 * (usize::from(node.position) + usize::from(node.velocity)))
			.sum()
	}
}

super::impl_direct_resource!(MotionDatabase, "MotionDatabase", references = motion_database_references);

/// Lists the clips the database samples, in database order.
fn motion_database_references(database: &MotionDatabase) -> Vec<&str> {
	database.clips.iter().map(|clip| clip.id.as_str()).collect()
}

/// The `MotionClip` struct places one clip's frames in its [`MotionDatabase`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct MotionClip {
	/// The clip's resource ID, which the clip pool loads it by.
	pub id: String,
	/// How many frames the clip has: one per frame interval from zero up to its duration, or, for a looping clip, up to
	/// but not including its duration.
	pub frames: u32,
	/// How many of its first frames a search can find: those with a whole trajectory left in the clip, or every frame
	/// of a looping clip.
	pub pickable: u32,
	/// The clip's duration in seconds.
	pub duration: f32,
	/// How a looping clip's next cycle moves from this one, which continues its ground root past its ends: the turn
	/// about +Y, then the X and Z translation, as `[x, z, yaw]` in the clip's object space. `None` for a clip that
	/// plays once.
	pub cycle: Option<[f32; 3]>,
}

/// The `MotionNode` struct names one node whose features a [`MotionDatabase`] compares.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct MotionNode {
	/// The node's name, unique in each clip's skeleton.
	pub name: String,
	/// Whether the node's position is a feature, three dimensions.
	pub position: bool,
	/// Whether the node's velocity is a feature, three dimensions after the position's.
	pub velocity: bool,
}
