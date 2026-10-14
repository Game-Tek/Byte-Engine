use math::{Point, UnitVector, collision::DynamicIntersection, collision::Intersection};

use crate::core::alley::PairBatches;

/// The `Contact` struct stores the world-space data required to resolve one collision.
#[derive(Debug)]
pub struct Contact {
	pub(crate) a: Side,
	pub(crate) b: Side,
	pub(crate) normal: UnitVector,
	/// How far the bodies are in each other at the time of the contact. Negative for bodies that are that far
	/// short of touching then.
	pub(crate) depth: f32,
	/// How far into the step the bodies touch, in seconds. Zero for bodies that touch where they are. For bodies
	/// that touch later, the contact holds them to the speed that brings them together as the step ends.
	pub(crate) toi: f32,
	/// Which of the contacts between its two bodies this is, counted from zero. Bodies that touch over a patch have
	/// one at each of up to four corners of it, and the solver starts each from what the one with its number
	/// needed in the last step.
	pub(crate) corner: u8,
}

impl Contact {
	/// Converts a static math intersection into a physics contact between bodies that touch where they are.
	pub(crate) fn from_static(a: usize, b: usize, intersection: Intersection) -> Self {
		Self {
			a: Side {
				object: a,
				point: intersection.point_on_a(),
			},
			b: Side {
				object: b,
				point: intersection.point_on_b(),
			},
			normal: intersection.normal(),
			depth: intersection.depth(),
			toi: 0.0,
			corner: 0,
		}
	}

	/// Converts a dynamic math intersection into a physics contact at the time the bodies touch.
	pub(crate) fn from_dynamic(a: usize, b: usize, intersection: DynamicIntersection) -> Self {
		let toi = intersection.toi();
		Self {
			toi,
			..Self::from_static(a, b, intersection.into_contact())
		}
	}
}

/// The `Side` struct identifies one body and its world-space contact point.
#[derive(Debug)]
pub struct Side {
	pub(crate) object: usize,
	pub(crate) point: Point,
}

/// The `Mobility` enum tells contact batching whether the contacts of a step can change a body.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mobility {
	/// Contacts change the body.
	Movable,
	/// Nothing changes the body, such as a static one.
	Fixed,
	/// The body sleeps, and contacts only change it if it wakes before they are batched. See
	/// [`Contacts::release`].
	Asleep,
}

/// The `ContactPart` struct holds the contacts one lane found in its share of a scan, with what batching them needs
/// to know about each, so that the lane works that out while it has the bodies at hand.
///
/// Add contacts with [`Self::find`], and give the part to [`Contacts::part_mut`] once the scan is done.
#[derive(Default)]
pub(crate) struct ContactPart {
	contacts: Vec<Contact>,
	/// For each contact, the slots of its two bodies, with [`PairBatches::FIXED`] added to the slot of a body that
	/// contacts do not change, such as a static or a sleeping one.
	bodies: Vec<[u32; 2]>,
	/// The indices in this part of the contacts with a sleeping body.
	asleep: Vec<u32>,
}

impl ContactPart {
	/// Adds the contacts `find` finds between two bodies. When it finds any, `bodies` is asked for the slot of each
	/// body, and whether contacts can change it.
	#[inline]
	pub(crate) fn find(&mut self, find: impl FnOnce(&mut Vec<Contact>), bodies: impl FnOnce() -> [(usize, Mobility); 2]) {
		let before = self.contacts.len();
		find(&mut self.contacts);
		// Most pairs that are tested turn out to be apart.
		if self.contacts.len() != before {
			let bodies = bodies();
			if bodies.iter().any(|(_, mobility)| *mobility == Mobility::Asleep) {
				self.asleep.extend(before as u32..self.contacts.len() as u32);
			}
			let bodies = bodies.map(|(slot, mobility)| {
				debug_assert!(
					slot < PairBatches::FIXED as usize,
					"Body slot is too large. The most likely cause is a body index above the 31-bit slot range."
				);
				let fixed = mobility != Mobility::Movable;
				slot as u32 | if fixed { PairBatches::FIXED } else { 0 }
			});
			for _ in before..self.contacts.len() {
				self.bodies.push(bodies);
			}
		}
	}

	/// Empties the part, ready for the contacts of the next step.
	pub(crate) fn clear(&mut self) {
		self.contacts.clear();
		self.bodies.clear();
		self.asleep.clear();
	}

	/// Returns the contacts in the order they were found.
	#[cfg(test)]
	pub(crate) fn into_contacts(self) -> Vec<Contact> {
		self.contacts
	}
}

/// The `Contacts` struct holds a physics step's contacts in the order they were found, as the parts lanes found them
/// in, so that gathering what every lane found moves no contact.
///
/// Fill the parts through [`Self::part_mut`], in the order a single scan would find their contacts, then call
/// [`Self::seal`]. After that a contact's index is its place in that order.
#[derive(Default)]
pub(crate) struct Contacts {
	parts: Vec<ContactPart>,
	/// The index of each part's first contact, followed by the number of contacts. Written by [`Self::seal`].
	starts: Vec<usize>,
}

impl Contacts {
	/// Returns the part at `part`, adding empty parts up to it. Parts keep their storage between steps.
	pub(crate) fn part_mut(&mut self, part: usize) -> &mut ContactPart {
		if self.parts.len() <= part {
			self.parts.resize_with(part + 1, Default::default);
		}
		&mut self.parts[part]
	}

	/// Numbers the contacts through the parts as they are now. Call it once every part is filled.
	pub(crate) fn seal(&mut self) {
		self.starts.clear();
		let mut start = 0;
		for part in &self.parts {
			self.starts.push(start);
			start += part.contacts.len();
		}
		self.starts.push(start);
	}

	/// Returns the number of contacts that were numbered by [`Self::seal`].
	pub(crate) fn len(&self) -> usize {
		self.starts.last().copied().unwrap_or(0)
	}

	/// Returns the contact at `index`.
	#[inline]
	pub(crate) fn get(&self, index: usize) -> &Contact {
		// There are as many parts as lanes, so looking through them takes less than a search would.
		let part = self.starts[1..].iter().take_while(|end| **end <= index).count();
		&self.parts[part].contacts[index - self.starts[part]]
	}

	/// Returns, for each contact in order, the slots of its two bodies, with [`PairBatches::FIXED`] added to the
	/// slot of a body that contacts never change. The contacts come as the runs the parts hold, one after the other.
	pub(crate) fn bodies(&self) -> impl Iterator<Item = &[[u32; 2]]> {
		self.parts.iter().map(|part| part.bodies.as_slice())
	}

	/// Lets the contacts in the first `parts` parts change the bodies that slept when they were found and that
	/// `awake` now says are awake. Call it for the bodies that wake before the contacts are batched.
	pub(crate) fn release(&mut self, parts: usize, awake: impl Fn(usize) -> bool) {
		for part in self.parts.iter_mut().take(parts) {
			for index in &part.asleep {
				for slot in &mut part.bodies[*index as usize] {
					// A contact with a sleeping body is between it and one that is awake, which is not fixed.
					if *slot & PairBatches::FIXED != 0 && awake((*slot & !PairBatches::FIXED) as usize) {
						*slot &= !PairBatches::FIXED;
					}
				}
			}
		}
	}

	/// Empties the parts from `part` on, which no lane filled in this step.
	pub(crate) fn clear_from(&mut self, part: usize) {
		for part in self.parts.iter_mut().skip(part) {
			part.clear();
		}
	}
}

#[cfg(test)]
mod tests {
	use math::{Sphere, collision::sphere_vs_sphere};

	use super::*;

	/// Returns a contact between the bodies in slots `a` and `b`.
	fn contact(a: usize, b: usize) -> Contact {
		let touching = sphere_vs_sphere(
			&Sphere::new(Point::origin(), 1.0),
			&Sphere::new(Point::new(1.0, 0.0, 0.0), 1.0),
		);
		Contact::from_static(a, b, touching.expect("the spheres overlap"))
	}

	#[test]
	fn contacts_are_numbered_through_their_parts_in_order() {
		let mut contacts = Contacts::default();
		// The part between the two others found nothing, as a lane with no overlapping bounds would.
		// The floor in slot 6 is a body that contacts never change.
		contacts.part_mut(2).find(
			|found| found.push(contact(4, 5)),
			|| [(4, Mobility::Movable), (5, Mobility::Movable)],
		);
		contacts.part_mut(2).find(
			|found| found.push(contact(6, 7)),
			|| [(6, Mobility::Fixed), (7, Mobility::Movable)],
		);
		// Bodies that touch at two places have two contacts, and bodies that are apart have none.
		contacts.part_mut(0).find(
			|found| found.extend([contact(0, 1), contact(2, 3)]),
			|| [(0, Mobility::Movable), (1, Mobility::Movable)],
		);
		contacts
			.part_mut(0)
			.find(|_| (), || [(8, Mobility::Movable), (9, Mobility::Movable)]);
		contacts.seal();

		assert_eq!(contacts.len(), 4);
		let firsts = |contacts: &Contacts| {
			(0..contacts.len())
				.map(|index| contacts.get(index).a.object)
				.collect::<Vec<_>>()
		};
		assert_eq!(firsts(&contacts), [0, 2, 4, 6]);
		assert_eq!(
			(0..contacts.len())
				.map(|index| contacts.get(index).b.object)
				.collect::<Vec<_>>(),
			[1, 3, 5, 7]
		);
		assert_eq!(
			contacts.bodies().flatten().copied().collect::<Vec<_>>(),
			[[0, 1], [0, 1], [4, 5], [6 | PairBatches::FIXED, 7]]
		);

		contacts.clear_from(1);
		contacts.seal();
		assert_eq!(contacts.len(), 2);
	}

	#[test]
	fn contacts_change_a_sleeping_body_once_it_is_released() {
		let mut contacts = Contacts::default();
		contacts.part_mut(0).find(
			|found| found.push(contact(0, 1)),
			|| [(0, Mobility::Movable), (1, Mobility::Asleep)],
		);
		contacts.part_mut(1).find(
			|found| found.push(contact(2, 3)),
			|| [(2, Mobility::Asleep), (3, Mobility::Movable)],
		);
		let bodies = |contacts: &Contacts| contacts.bodies().flatten().copied().collect::<Vec<_>>();
		assert_eq!(bodies(&contacts), [[0, 1 | PairBatches::FIXED], [2 | PairBatches::FIXED, 3]]);

		// Only the body that woke is released, and only in the parts that were found.
		contacts.release(2, |slot| slot == 1);
		assert_eq!(bodies(&contacts), [[0, 1], [2 | PairBatches::FIXED, 3]]);
	}
}
