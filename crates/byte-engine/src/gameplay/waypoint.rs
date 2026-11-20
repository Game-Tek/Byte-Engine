//! Waypoints: views of a level that tools go to, such as the editor's Cmd+G.

use serde::Deserialize;

/// The `Waypoint` struct marks an entity's transform as a view of the level a tool can go to, so a designer reaches
/// the relevant spots of a large level quickly.
///
/// A level places one with the `waypoint` kind, next to a `transform` for the view and a [`Name`](super::Name) to list
/// it by; see [waypoints](/docs/reference/levels#waypoints). It changes nothing in the game.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Waypoint {}
